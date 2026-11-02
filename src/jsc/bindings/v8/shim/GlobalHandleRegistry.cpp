#include "GlobalHandleRegistry.h"
#include "GlobalInternals.h"
#include "HandleScopeBuffer.h"
#include "ScriptExecutionContext.h"
#include "ZigGlobalObject.h"
#include "napi_handle_scope.h"
#include <wtf/TZoneMallocInlines.h>

namespace v8 {
namespace shim {

WTF_MAKE_STRUCT_TZONE_ALLOCATED_IMPL(GlobalHandleLists);
WTF_MAKE_TZONE_ALLOCATED_IMPL(GlobalHandleRegistry);

GlobalHandleRegistry::GlobalHandleRegistry(JSC::VM& vm)
    : m_vm(vm)
{
}

// The lists that still have live handles stay allocated with their storage: a v8::Global that outlived this VM has the address of one of those handles, and no later handle may get it.
GlobalHandleRegistry::~GlobalHandleRegistry() = default;

GlobalHandleRegistry& GlobalHandleRegistry::ensure(JSC::VM& vm)
{
    auto& registry = WebCore::clientData(vm)->v8GlobalHandleRegistry;
    if (!registry) {
        registry = makeUnique<GlobalHandleRegistry>(vm);
    }
    return *registry;
}

void GlobalHandleRegistry::addRealm(GlobalInternals* internals)
{
    m_realms.removeAllMatching([](auto& realm) {
        return !realm;
    });
    m_realms.append(JSC::Weak<GlobalInternals>(internals));
    internals->setHasDisposedGlobalHandles(m_realmsHaveDisposedHandles);
}

void GlobalHandleRegistry::dispose(Handle* handle)
{
    GlobalHandleLists* lists = m_live.take(handle);
    // Not a live v8::Global of this VM: a root, the handle of another VM, or one that is disposed already.
    if (!lists) {
        return;
    }
    lists->weakParameters.remove(handle);
    lists->liveCount--;
    if (!lists->buffer) {
        if (!lists->liveCount) {
            delete lists;
        }
        return;
    }
    // The owner of the buffer can be destroyed already. The destructor of the buffer deletes the lists or gives them its storage.
    if (m_vm.heap.isShuttingDown()) {
        return;
    }
    lists->disposed.append(handle);
    if (lists->disposed.size() == 1) {
        m_listsWithDisposedHandles.append(lists);
    }
    // Else earlier handles wait too. What releases them releases this one: a scope that closes, or the queued task.
    if (!m_realmsHaveDisposedHandles) {
        releaseDisposedHandles(LockMode::Try);
    }
}

void GlobalHandleRegistry::setWeakParameter(Handle* handle, void* parameter)
{
    if (auto* lists = m_live.get(handle)) {
        lists->weakParameters.set(handle, parameter);
    }
}

void* GlobalHandleRegistry::takeWeakParameter(Handle* handle)
{
    auto* lists = m_live.get(handle);
    return lists ? lists->weakParameters.take(handle) : nullptr;
}

void GlobalHandleRegistry::didDestroyBuffer(GlobalHandleLists& lists, WTF::SegmentedVector<Handle, 16>&& storage)
{
    m_listsWithDisposedHandles.removeFirst(&lists);
    if (!lists.liveCount) {
        delete &lists;
        return;
    }
    // A live v8::Global still points into this storage. It is freed with the lists, when the last of them is disposed.
    lists.storage = WTF::move(storage);
    lists.buffer = nullptr;
    lists.disposed.clear();
    lists.free.clear();
}

NEVER_INLINE void GlobalHandleRegistry::didCloseScope(JSC::VM& vm)
{
    ifExists(vm)->releaseDisposedHandles(LockMode::Wait);
}

void GlobalHandleRegistry::releaseDisposedHandles(LockMode lockMode)
{
    if (!m_listsWithDisposedHandles.isEmpty() && !hasOpenScopeWithV8Handles()) {
        m_listsWithDisposedHandles.removeAllMatching([&](GlobalHandleLists* lists) {
            auto& lock = lists->buffer->m_owner->cellLock();
            // DisposeGlobal never waits: a static v8::Global is destroyed at exit, when the thread that holds the lock can be gone.
            auto locker = lockMode == LockMode::Wait ? WTF::Locker { lock } : WTF::Locker<JSC::JSCellLock>::tryLock(lock);
            if (!locker) {
                return false;
            }
            for (Handle* handle : lists->disposed) {
                *handle = Handle();
            }
            lists->free.appendVector(lists->disposed);
            lists->disposed.shrink(0);
            return true;
        });
        if (!m_listsWithDisposedHandles.isEmpty()) {
            queueReleaseTask();
        }
    }
    // Each realm has a copy, so that HandleScopeBuffer::close() reads one byte of its own realm.
    bool hasDisposedHandles = !m_listsWithDisposedHandles.isEmpty();
    if (hasDisposedHandles == m_realmsHaveDisposedHandles) {
        return;
    }
    m_realmsHaveDisposedHandles = hasDisposedHandles;
    for (auto& realm : m_realms) {
        if (auto* internals = realm.get()) {
            internals->setHasDisposedGlobalHandles(hasDisposedHandles);
        }
    }
}

// Whether a realm of the VM has an open scope with V8 handles. A Local of it can point into a disposed handle.
bool GlobalHandleRegistry::hasOpenScopeWithV8Handles() const
{
    for (auto& realm : m_realms) {
        auto* internals = realm.get();
        if (!internals) {
            continue;
        }
        for (auto* scope = internals->isolate()->globalObject()->m_currentHandleScopeImpl.get(); scope; scope = scope->parent()) {
            if (scope->v8Handles()) {
                return true;
            }
        }
    }
    return false;
}

void GlobalHandleRegistry::queueReleaseTask()
{
    auto* globalObject = defaultGlobalObject(m_vm);
    auto* context = globalObject ? globalObject->scriptExecutionContext() : nullptr;
    // A VM that exits runs no more tasks.
    if (m_releaseTaskIsQueued || !context || globalObject->isShuttingDown()) {
        return;
    }
    m_releaseTaskIsQueued = true;
    context->postTask([](WebCore::ScriptExecutionContext& context) {
        if (auto* registry = ifExists(context.vm())) {
            registry->m_releaseTaskIsQueued = false;
            registry->releaseDisposedHandles(LockMode::Try);
        }
    });
}

} // namespace shim
} // namespace v8
