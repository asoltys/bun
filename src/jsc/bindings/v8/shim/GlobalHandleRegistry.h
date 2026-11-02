#pragma once

#include "BunClientData.h"
#include "Handle.h"
#include <JavaScriptCore/Weak.h>
#include <wtf/HashMap.h>
#include <wtf/Noncopyable.h>
#include <wtf/SegmentedVector.h>
#include <wtf/TZoneMalloc.h>
#include <wtf/Vector.h>

namespace v8 {
namespace shim {

class GlobalHandleRegistry;
class GlobalInternals;
class HandleScopeBuffer;

// The handles of the v8::Globals that one HandleScopeBuffer made (GlobalInternals::globalHandles(), one for each realm), by state.
struct GlobalHandleLists {
    WTF_MAKE_STRUCT_TZONE_ALLOCATED(GlobalHandleLists);

    GlobalHandleLists(GlobalHandleRegistry& registry, HandleScopeBuffer* buffer)
        : registry(registry)
        , buffer(buffer)
    {
    }

    GlobalHandleRegistry& registry;
    // Null once the buffer is destroyed. Nothing visits the handles from then on.
    HandleScopeBuffer* buffer;
    size_t liveCount { 0 };
    WTF::Vector<Handle*> disposed;
    WTF::Vector<Handle*> free;
    // What MakeWeak() stored for a live handle, which ClearWeak() returns.
    WTF::HashMap<Handle*, void*> weakParameters;
    // The handles of the buffer, once it is destroyed while some of them are live.
    WTF::SegmentedVector<Handle, 16> storage;
};

// The v8::Global handles of one JSC::VM. JSVMClientData owns it, so it is valid while the heap shuts down and runs the last finalizers.
class GlobalHandleRegistry {
    WTF_MAKE_TZONE_ALLOCATED(GlobalHandleRegistry);
    WTF_MAKE_NONCOPYABLE(GlobalHandleRegistry);

public:
    explicit GlobalHandleRegistry(JSC::VM&);
    ~GlobalHandleRegistry();

    // Null until a realm of the VM uses the V8 API.
    static GlobalHandleRegistry* ifExists(JSC::VM& vm) { return WebCore::clientData(vm)->v8GlobalHandleRegistry.get(); }
    static GlobalHandleRegistry& ensure(JSC::VM&);

    // An open scope of any realm of the VM can have a Local that points into the handle of a v8::Global.
    void addRealm(GlobalInternals*);

    void add(Handle* handle, GlobalHandleLists& lists)
    {
        m_live.add(handle, &lists);
        lists.liveCount++;
    }
    // These three ignore a handle that is not a live v8::Global of this VM. They read no cell while the heap shuts down.
    void dispose(Handle*);
    void setWeakParameter(Handle*, void* parameter);
    void* takeWeakParameter(Handle*);

    // Called by the destructor of a buffer that has lists, with the buffer's handles.
    void didDestroyBuffer(GlobalHandleLists&, WTF::SegmentedVector<Handle, 16>&& storage);
    // Called by HandleScopeBuffer::close() in a realm that hasDisposedGlobalHandles().
    static void didCloseScope(JSC::VM&);

private:
    enum class LockMode : bool { Try,
        Wait };

    // Clears the disposed handles and makes them free, unless a Local can still point into one.
    void releaseDisposedHandles(LockMode);
    bool hasOpenScopeWithV8Handles() const;
    void queueReleaseTask();

    JSC::VM& m_vm;
    // Each live handle, and the lists of the buffer that made it.
    WTF::HashMap<Handle*, GlobalHandleLists*> m_live;
    WTF::Vector<JSC::Weak<GlobalInternals>> m_realms;
    // The lists that have a buffer and disposed handles.
    WTF::Vector<GlobalHandleLists*> m_listsWithDisposedHandles;
    // What GlobalInternals::hasDisposedGlobalHandles() of each realm says.
    bool m_realmsHaveDisposedHandles { false };
    bool m_releaseTaskIsQueued { false };
};

} // namespace shim
} // namespace v8
