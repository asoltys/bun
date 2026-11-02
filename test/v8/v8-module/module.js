const path = require("node:path");
const { Worker, parentPort } = require("node:worker_threads");

module.exports = debugMode => {
  const nativeModule = require(`./build/${debugMode ? "Debug" : "Release"}/v8tests`);

  let nodeCollectGarbage;
  function collectGarbage() {
    if (process.isBun) return Bun.gc(true);
    if (!nodeCollectGarbage) {
      // Node only has gc() with --expose-gc, which can still be turned on here.
      require("node:v8").setFlagsFromString("--expose-gc");
      nodeCollectGarbage = require("node:vm").runInNewContext("gc");
    }
    nodeCollectGarbage();
  }
  const nextTurn = () => new Promise(resolve => setImmediate(resolve));
  const countCollected = refs => refs.filter(ref => ref.deref() === undefined).length;
  // A WeakRef keeps its target alive until the job that created or read it ends, so each collection runs in a later turn.
  async function collect(refs, enough) {
    let collected = 0;
    for (let turn = 0; turn < 10 && collected < enough; turn++) {
      await nextTurn();
      collectGarbage();
      collected = countCollected(refs);
    }
    return collected;
  }
  // The counts go to stderr, and to stdout only when they are too low, because the test compares the stdout of Node and Bun.
  function report(description, state, number, count, expected) {
    const counts = `${number} of ${count} ${state}`;
    console.error(`${description}: ${counts}`);
    console.log(`${description}:`, number >= expected ? state : `only ${counts}`);
  }
  // Returns before the first collection, so that only a Global can still refer to the objects.
  function holdNewObjects(count) {
    const refs = [];
    for (let i = 0; i < count; i++) {
      const object = { i };
      refs.push(new WeakRef(object));
      nativeModule.global_hold(object);
    }
    return refs;
  }
  // Runs a function of this module in a Worker, which main.js starts like the main thread. Resolves with what the Worker posted, once it has exited.
  function runInWorker(name, ...args) {
    const { promise, resolve, reject } = Promise.withResolvers();
    const argv = [name, JSON.stringify(args), "null", debugMode ? "debug" : "release"];
    const worker = new Worker(path.join(__dirname, "main.js"), { argv });
    let message;
    worker.on("message", value => (message = value));
    worker.on("error", reject);
    worker.on("exit", code =>
      code === 0 ? resolve(message) : reject(new Error(`the Worker exited with code ${code}`)),
    );
    return promise;
  }

  return {
    ...nativeModule,

    test_v8_function_script_origin() {
      function namedFunction(a, b) {
        return a + b;
      }
      const arrowFunction = (a, b) => a + b;
      class DefaultConstructor {
        method() {}
      }
      class ExplicitConstructor {
        constructor() {}
      }
      class Derived extends DefaultConstructor {}
      const AnonymousClass = class {};
      const cases = [
        ["named function", namedFunction],
        ["arrow function", arrowFunction],
        ["class with default constructor", DefaultConstructor],
        ["class with explicit constructor", ExplicitConstructor],
        ["derived class with default constructor", Derived],
        ["anonymous class expression", AnonymousClass],
        ["class method", DefaultConstructor.prototype.method],
        ["builtin function", Array.prototype.map],
        ["bound function", namedFunction.bind(null)],
        ["native addon function", nativeModule.get_function_script_origin],
      ];
      for (const [description, fn] of cases) {
        console.log(`====== ${description}`);
        const { file, line, column } = nativeModule.get_function_script_origin(fn);
        console.log("file:", typeof file === "string" ? path.basename(file) : file);
        console.log("file is absolute:", typeof file === "string" && path.isAbsolute(file));
        console.log("line:", line, "column:", column);
      }
    },

    test_v8_value_to_int32() {
      const cases = [
        ["int", 42],
        ["negative int", -7],
        ["fraction", 3.99],
        ["negative fraction", -3.99],
        ["INT32_MAX + 1", 2 ** 31],
        ["UINT32_MAX + 6", 2 ** 32 + 5],
        ["INT32_MIN - 1", -(2 ** 31) - 1],
        ["NaN", NaN],
        ["Infinity", Infinity],
        ["numeric string", "123"],
        ["hex string", "0x10"],
        ["true", true],
        ["null", null],
        ["undefined", undefined],
        ["object with valueOf", { valueOf: () => 77.5 }],
        [
          "object with throwing valueOf",
          {
            valueOf() {
              throw new Error("valueOf threw");
            },
          },
        ],
      ];
      for (const [description, value] of cases) {
        console.log(`====== ${description}`);
        try {
          console.log("result:", nativeModule.perform_to_int32(value));
        } catch (e) {
          console.log("threw:", e.message);
        }
      }
    },

    // Bun only: 48k collected instances with 128 internal fields each. Their
    // field storage is about 2 KB per instance, 100 MiB if none of it is freed.
    test_v8_internal_field_object_leak() {
      nativeModule.create_objects_with_internal_fields(256, 128);
      Bun.gc(true);
      const before = process.memoryUsage.rss();
      for (let i = 0; i < 24; i++) {
        nativeModule.create_objects_with_internal_fields(2000, 128);
        Bun.gc(true);
      }
      const after = process.memoryUsage.rss();
      console.log(JSON.stringify({ deltaMiB: (after - before) / 1024 / 1024 }));
    },

    test_v8_global() {
      console.log("global initial value =", nativeModule.global_get());

      nativeModule.global_set(123);
      console.log("global after setting to 123 =", nativeModule.global_get());

      nativeModule.global_set({ foo: 5, bar: ["one", "two", "three"] });
      if (process.isBun) {
        Bun.gc(true);
      }
      console.log("global after setting to object =", JSON.stringify(nativeModule.global_get()));

      nativeModule.global_set(true);
      console.log("global after setting to true =", nativeModule.global_get());
    },

    test_v8_global_reset_releases_value() {
      const count = 1000;
      // Leaves room for a few objects that something other than a Global keeps alive.
      const enough = count - 10;
      function resetInTheSameCall() {
        const refs = [];
        for (let i = 0; i < count; i++) {
          const object = { i };
          refs.push(new WeakRef(object));
          if (nativeModule.global_hold_and_reset(object) !== object) {
            throw new Error("global_hold_and_reset did not return the value of its Global");
          }
        }
        return refs;
      }
      (async () => {
        const sameCall = resetInTheSameCall();
        const laterCall = holdNewObjects(count);
        const description = "values of Globals reset in the call that created them";
        report(description, "collected", await collect(sameCall, enough), count, enough);
        // The collections above ran while these Globals held their values.
        report("values of Globals not reset yet", "alive", count - countCollected(laterCall), count, count);
        nativeModule.global_reset_held();
        report("values of Globals reset in a later call", "collected", await collect(laterCall, enough), count, enough);
      })();
    },

    test_v8_global_reset_keeps_locals_and_return_values() {
      let othersHeldTheirValues = true;
      // One of the Globals that the native function creates for its arguments could take the handle that the stored Global gave back.
      const callWithOthers = name => {
        const others = Array.from({ length: 16 }, (_, i) => ({ name: `other ${i}` }));
        const result = nativeModule[name](...others);
        const held = nativeModule.global_held();
        nativeModule.global_reset_held();
        if (held.length !== others.length || !held.every((value, i) => value === others[i])) {
          othersHeldTheirValues = false;
        }
        return result;
      };

      // Only the Global refers to the object.
      nativeModule.global_store({ name: "first" });
      console.log("returned local =", callWithOthers("global_local_outlives_reset").name);

      const makeValues = [
        () => ({ name: "second" }),
        () => "a string",
        () => 12.5,
        () => 7,
        () => undefined,
        () => null,
        () => true,
        () => false,
      ];
      for (const name of ["global_return_local_then_reset", "global_return_global_then_reset"]) {
        // Each call gets a new object, so that only the Global refers to it.
        const returned = makeValues.map(makeValue => {
          nativeModule.global_store(makeValue());
          return String(JSON.stringify(callWithOthers(name)));
        });
        console.log(`${name} returned`, returned.join(", "));
      }

      nativeModule.global_store({ name: "third" });
      const nested = nativeModule.global_local_outlives_nested_reset(() => {
        nativeModule.global_store({ name: "fourth" });
        nativeModule.global_store({ name: "fifth" });
      });
      console.log("returned after nested calls reset the Global =", nested.name);
      console.log("the Global then held", JSON.stringify(nativeModule.global_return_global_then_reset()));
      console.log("the other Globals held their values =", othersHeldTheirValues);
    },

    test_v8_global_reset_primitives() {
      // A round can get the handles that the Globals of an earlier round gave back.
      for (let round = 0; round < 5; round++) {
        nativeModule.global_primitives_round(round);
      }
      nativeModule.global_primitives_round(-1);
      nativeModule.global_hold(undefined, undefined, null, true, false);
      const handles = new Set(nativeModule.global_held_addresses());
      nativeModule.global_reset_held();
      console.log("Globals of undefined, undefined, null, true and false have", handles.size, "handles");
    },

    test_v8_global_reuses_handles() {
      const handles = new Set();
      for (let round = 0; round < 1000; round++) {
        handles.add(nativeModule.global_hold_briefly({ round }));
      }
      console.error(`${handles.size} handles for 1000 Globals that were created and reset one after another`);
      console.log("handles for 1000 Globals, one at a time:", handles.size <= 16 ? "at most 16" : handles.size);
    },

    test_v8_global_on_another_thread() {
      nativeModule.global_use_on_another_thread({ name: "made on the JS thread" });
    },

    test_v8_global_reset_in_uv_timer() {
      const count = 1000;
      const fired = new Uint8Array(new ArrayBuffer(1));
      (async () => {
        const refs = holdNewObjects(count);
        nativeModule.global_reset_held_in_uv_timer(fired.buffer);
        // Nothing calls into the addon from here on: the end of a native call that uses the V8 API would release the handles too.
        while (!fired[0]) await nextTurn();
        const description = "values of Globals reset in a uv_timer callback that creates no Local";
        report(description, "collected", await collect(refs, count - 10), count, count - 10);
      })();
    },

    // Bun only: after a Worker has exited, a Global that it left set points to a handle of an isolate that is gone.
    test_v8_global_left_by_workers() {
      (async () => {
        await Promise.all([
          runInWorker("global_leave_new_objects", 1024),
          runInWorker("global_leave_new_objects", 1024),
        ]);
        const left = new Set(nativeModule.global_left_addresses());
        const objects = Array.from({ length: 8000 }, (_, i) => ({ i }));
        for (let i = 0; i < objects.length; i += 1000) {
          nativeModule.global_hold(...objects.slice(i, i + 1000));
        }
        const atLeftAddresses = nativeModule.global_held_addresses().filter(address => left.has(address)).length;
        nativeModule.global_reset_left();
        // A Global that the reset above disposed by mistake gave its handle back, and one of these takes it.
        const later = Array.from({ length: left.size }, (_, i) => ({ later: i }));
        nativeModule.global_hold(...later);
        collectGarbage();
        const held = nativeModule.global_held();
        const wrong = held.filter((value, i) => value !== (objects[i] ?? later[i - objects.length])).length;
        nativeModule.global_reset_held();
        console.log("Globals that Workers left when they exited:", left.size);
        console.log("Globals with the handle address of one of them:", atLeftAddresses);
        console.log("Globals with a wrong value after those that the Workers left were reset:", wrong);
      })();
    },

    global_leave_new_objects(count) {
      nativeModule.global_leave(...Array.from({ length: count }, (_, i) => ({ i })));
    },

    // Bun only: in Node, a thread that disposes a handle of another thread's isolate races with that thread.
    test_v8_global_of_another_isolate() {
      (async () => {
        let refs;
        (() => {
          const held = { name: "held" };
          const stored = { name: "stored" };
          refs = [new WeakRef(held), new WeakRef(stored)];
          nativeModule.global_hold(held);
          nativeModule.global_store(stored);
        })();
        const worker = await runInWorker("global_reset_stored_in_worker", nativeModule.global_stored_address());
        await nextTurn();
        collectGarbage();
        const names = refs.map(ref => ref.deref()?.name).join(", ");
        console.log("the Worker's Globals held their values:", worker.heldTheirValues);
        console.log("a Global of the Worker took the handle of the main thread's Global:", worker.tookTheHandle);
        console.log("the values of the main thread's Globals that the Worker reset:", names);
      })();
    },

    global_reset_stored_in_worker(addressOfStored) {
      // Returns before the collection, so that only the Globals keep the objects alive.
      function makeGlobals() {
        const objects = Array.from({ length: 10 }, (_, i) => ({ name: `worker ${i}` }));
        // This thread's isolate has no handles yet, and the Global that the main thread holds is reset.
        nativeModule.global_reset_held();
        // The first Global gives this thread's isolate its handles, where the next reset looks for the one of the stored Global.
        nativeModule.global_hold(objects[0]);
        nativeModule.global_store(objects[1]);
        nativeModule.global_hold(...objects.slice(2));
        return [nativeModule.global_stored_address(), ...nativeModule.global_held_addresses()];
      }
      (async () => {
        const addresses = makeGlobals();
        await nextTurn();
        collectGarbage();
        const names = nativeModule.global_held().map(value => value?.name);
        nativeModule.global_reset_held();
        parentPort.postMessage({
          heldTheirValues: names.join() === [0, 2, 3, 4, 5, 6, 7, 8, 9].map(i => `worker ${i}`).join(),
          tookTheHandle: addresses.includes(addressOfStored),
        });
      })();
    },

    // Bun only: Node cannot reset a Global once its isolate is disposed. Here a destructor resets two after the VM of their thread is destroyed.
    test_v8_global_reset_by_destructor(ofThread) {
      (async () => {
        if (ofThread) {
          await runInWorker("global_hold_until_destructor", { name: "held by a Worker" }, true);
          // The Worker's exit event does not wait for the thread to exit.
          while (nativeModule.global_count_holders_destroyed() === 0) await nextTurn();
          console.log("the thread of the Worker has exited");
        } else {
          nativeModule.global_hold_until_destructor({ name: "held by the main thread" }, false);
          console.log("the main thread is done");
        }
      })();
    },

    test_v8_function_template() {
      const f = nativeModule.create_function_with_data();
      if (process.isBun) {
        Bun.gc(true);
      }
      console.log(f());
    },

    test_v8_function_template_set_class_name() {
      const f = nativeModule.test_v8_function_template_set_class_name();
      console.log("f.name =", f.name);
    },

    test_v8_define_own_property() {
      const obj = nativeModule.test_v8_define_own_property();
      for (const key of ["writable", "readonly", "hidden"]) {
        const d = Object.getOwnPropertyDescriptor(obj, key);
        console.log(
          key,
          "=",
          d.value,
          "writable =",
          d.writable,
          "enumerable =",
          d.enumerable,
          "configurable =",
          d.configurable,
        );
      }
      console.log("enumerable keys =", JSON.stringify(Object.keys(obj).sort()));
    },

    test_v8_bigint() {
      const big = nativeModule.test_v8_bigint();
      console.log("typeof =", typeof big);
      console.log("value =", big.toString());
      console.log("is expected =", big === 123456789012345n);
    },

    test_v8_prototype_template() {
      const inst = nativeModule.test_v8_prototype_template();
      console.log("protoMethod() =", inst.protoMethod());
      console.log("nativeProp =", inst.nativeProp);
      console.log("protoMethod is own =", Object.prototype.hasOwnProperty.call(inst, "protoMethod"));
    },

    test_v8_return_value_from_inner_scope() {
      // The native functions invoke this after their inner handle scope closed,
      // while the callback is still on the stack, so the collection runs in the
      // window where only the preserved return value keeps the cell alive.
      // No-op under Node so the output matches.
      const maybeGc = () => {
        if (process.isBun) {
          Bun.gc(true);
        }
      };
      console.log("string =", nativeModule.return_string_from_inner_scope(1, maybeGc));
      console.log("number =", nativeModule.return_heap_number_from_inner_scope(1, maybeGc));
      console.log("element =", nativeModule.return_array_element_from_iterate(["one", "two", "three"], maybeGc));
      console.log("accessor =", nativeModule.return_accessor_value_from_inner_scope());
    },

    test_v8_function_call() {
      function target(a, b, c) {
        if (arguments.length === 0) return this && this.tag;
        return a + 1;
      }
      const err = nativeModule.test_v8_function_call(target);
      if (err) throw new Error(err);
    },

    test_v8_map() {
      const map = new Map();
      const ret = nativeModule.test_v8_map(map);
      if (typeof ret === "string") throw new Error(ret);
      console.log("size =", map.size);
      console.log("has a =", map.has("a"));
      console.log("has b =", map.has("b"));
      console.log("get b =", map.get("b"));
    },

    test_v8_exception() {
      try {
        nativeModule.test_v8_exception();
        console.log("did not throw");
      } catch (e) {
        console.log("caught", e.constructor.name, e.message);
      }
    },

    test_v8_exception_empty_message() {
      for (const e of nativeModule.test_v8_exception_empty_message()) {
        console.log(e.constructor.name, "message =", JSON.stringify(e.message), "own =", Object.hasOwn(e, "message"));
      }
    },

    test_v8_throw_error_empty_message() {
      try {
        nativeModule.test_v8_throw_error_empty_message();
        console.log("did not throw");
      } catch (e) {
        const own = Object.hasOwn(e, "message");
        console.log("caught", e.constructor.name, "message =", JSON.stringify(e.message), "own =", own);
      }
    },

    test_v8_new_values_with_pending_exception() {
      try {
        nativeModule.test_v8_new_values_with_pending_exception();
        console.log("did not throw");
      } catch (e) {
        console.log("caught", e.message);
      }
    },

    print_native_function() {
      nativeModule.print_values_from_js(nativeModule.create_function_with_data());
    },

    call_function_with_weird_this_values() {
      for (const thisValue of [null, undefined, 5, "abc"]) {
        const ret = nativeModule.return_this.call(thisValue);
        console.log("typeof =", typeof ret);
        if (ret == globalThis) {
          console.log("returned globalThis");
        } else if (ret instanceof String) {
          console.log("returned boxed String:", ret.toString());
        } else {
          console.log("returned", ret);
        }
        console.log("constructor is", ret.constructor.name);
      }
    },

    call_function_bare_through_closure() {
      const { return_this } = nativeModule;
      // return_this is captured by keep, so the bare call below is resolved through the closure's
      // scope object, which JSC leaves in the call's this slot. The callback must still see the
      // sloppy-mode receiver (globalThis), never that scope object.
      function keep() {
        return return_this;
      }
      console.log("bare call returned globalThis:", return_this() === globalThis);
      keep();
    },

    native_accessor_holder_on_property_access() {
      const obj = nativeModule.create_object_with_holder_accessor();
      console.log("getter holder is the object:", obj.holder === obj);
      obj.holder = 1;
      console.log("setter holder is the object:", nativeModule.global_get() === obj);
    },

    // Bun only: V8 itself installs a native data property as a data property, so Bun is the one
    // runtime where its getter and setter exist as functions that can be called with any receiver.
    native_accessor_holder_for_weird_receivers() {
      const obj = nativeModule.create_object_with_holder_accessor();
      const { get, set } = Object.getOwnPropertyDescriptor(obj, "holder");
      // get and set are captured by keep, so the bare calls below are resolved through the
      // closure's scope object, which JSC leaves in the call's this slot.
      function keep() {
        return [get, set];
      }
      const describe = value => {
        if (value === globalThis) return "globalThis";
        if (value === obj) return "the object";
        if (value instanceof Number) return "a Number object";
        return typeof value;
      };
      const holderSeenBySetter = receiver => {
        nativeModule.global_set(undefined);
        set.call(receiver, 1);
        return nativeModule.global_get();
      };
      console.log("bare getter():", describe(get()));
      console.log("getter.call(undefined):", describe(get.call(undefined)));
      console.log("getter.call(null):", describe(get.call(null)));
      console.log("getter.call(5):", describe(get.call(5)));
      console.log("getter.call(obj):", describe(get.call(obj)));
      nativeModule.global_set(undefined);
      set(1);
      console.log("bare setter():", describe(nativeModule.global_get()));
      console.log("setter.call(undefined):", describe(holderSeenBySetter(undefined)));
      console.log("setter.call(null):", describe(holderSeenBySetter(null)));
      console.log("setter.call(5):", describe(holderSeenBySetter(5)));
      console.log("setter.call(obj):", describe(holderSeenBySetter(obj)));
      keep();
    },

    test_uv_queue_work_on_current_event_loop() {
      const rc = nativeModule.queue_work_and_call((...args) => console.log("after_work_cb", ...args));
      console.log("uv_queue_work", rc);
    },

    test_v8_object_get_set_exceptions() {
      for (const key of [0, "key"]) {
        for (const access of ["get", "set"]) {
          const name = `perform_object_${access}_by_${typeof key == "number" ? "index" : "key"}`;
          const nativeFunction = nativeModule[name];
          if (typeof nativeFunction !== "function") throw new Error(name);

          const normal = { [key]: 5 };
          const accessor = {};
          Object.defineProperty(accessor, key, {
            [access](...args) {
              throw new Error("exception from accessor");
            },
          });
          const proxy = new Proxy(
            {},
            {
              [access](...args) {
                throw new Error("exception from proxy");
              },
            },
          );
          const readonly = {};
          Object.defineProperty(readonly, key, { configurable: true, writable: false, enumerable: true, value: "bar" });

          for (const [object, description] of [
            [normal, "plain object"],
            [accessor, "object with accessor that throws"],
            [proxy, "proxy with handler that throws"],
            [readonly, "plain object with readonly property"],
          ]) {
            console.log(`======\n${access} ${key} on ${description}`);
            try {
              nativeFunction(object, key, "foo");
              console.log("did not throw");
              if (object === normal || object === readonly) console.log("now value is", object[key]);
            } catch (e) {
              console.log(`threw: ${e.message}`);
            }
          }
        }
      }
    },
  };
};
