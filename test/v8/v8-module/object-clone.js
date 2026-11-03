// The drivers of v8::Object::Clone(), apart from the others: `bun build --compile --aot` builds them without any
// built-in module, so that v8.test.ts can run them as compiled code too.
module.exports = nativeModule => ({
  test_v8_object_clone() {
    let getterCalls = 0;
    const getter = () => {
      getterCalls++;
      return "value from the getter";
    };
    const setter = value => {};
    const symbol = Symbol("symbol key");
    const nested = { shared: true };
    class Base {
      method() {
        return "method on the prototype";
      }
    }
    // One entry per own property: key, what it holds, and a letter for each of writable, enumerable and configurable.
    const describe = object =>
      Reflect.ownKeys(object)
        .map(key => {
          const d = Object.getOwnPropertyDescriptor(object, key);
          const holds =
            "value" in d
              ? JSON.stringify(d.value)
              : `accessor (same getter: ${d.get === getter}, same setter: ${d.set === setter})`;
          const attributes = (d.writable ? "w" : "-") + (d.enumerable ? "e" : "-") + (d.configurable ? "c" : "-");
          return `${key === symbol ? "[symbol]" : String(key)} = ${holds} ${attributes}`;
        })
        .join(", ");

    // The properties added after the literal do not all fit in the object itself.
    const withData = { number: 1, string: "two", nested };
    for (const name of ["third", "fourth", "fifth", "sixth"]) withData[name] = name;

    const instance = new Base();
    instance.data = "data value";
    instance.nested = nested;
    instance[symbol] = "symbol value";
    Object.defineProperty(instance, "hidden", { value: "non-enumerable", writable: true, configurable: true });
    Object.defineProperty(instance, "accessor", { get: getter, set: setter, enumerable: true, configurable: true });

    // Bun only copies an object that has no index key, is no prototype, and has few enough properties to share their layout.
    const someDeleted = { nested };
    for (let i = 0; i < 60; i++) someDeleted["p" + i] = i;
    for (let i = 0; i < 60; i += 3) delete someDeleted["p" + i];

    const frozen = Object.freeze({ data: "data value", nested });
    const withoutPrototype = Object.assign(Object.create(null), { data: "data value", nested });

    for (const [description, original] of [
      ["object with data properties", withData],
      ["class instance with data, non-enumerable, accessor and symbol properties", instance],
      ["object with 60 properties, some deleted", someDeleted],
      ["frozen object", frozen],
      ["object without a prototype", withoutPrototype],
    ]) {
      console.log(`====== ${description}`);
      const before = describe(original);
      const copy = nativeModule.perform_object_clone(original);
      const secondCopy = nativeModule.perform_object_clone(original);
      console.log("copy is another object:", copy !== original);
      console.log("same prototype:", Object.getPrototypeOf(copy) === Object.getPrototypeOf(original));
      console.log("own properties of the copy:", describe(copy));
      console.log("same as the original's:", describe(copy) === before);
      console.log(
        "extensible, sealed, frozen:",
        Object.isExtensible(copy),
        Object.isSealed(copy),
        Object.isFrozen(copy),
      );
      console.log("nested object is shared:", copy.nested === nested);
      if (original === instance) {
        console.log("getter calls during Clone():", getterCalls);
        console.log("accessor read on the copy:", copy.accessor);
        console.log("method called on the copy:", copy.method());
      }

      // Writes to one of the three objects must not show up in the other two.
      const keys = Reflect.ownKeys(original);
      const [first, last] = [keys[0], keys[keys.length - 1]];
      copy[first] = copy[last] = "changed on the copy";
      copy.addedToCopy = true;
      delete copy.nested;
      const copyAfter = describe(copy);
      console.log("copy after writing to it:", copyAfter);
      console.log("original is unchanged:", describe(original) === before);
      console.log("second copy is unchanged:", describe(secondCopy) === before);
      original[first] = original[last] = "changed on the original";
      original.addedToOriginal = true;
      console.log("copy is unchanged by writing to the original:", describe(copy) === copyAfter);
      console.log("second copy is unchanged by writing to the original:", describe(secondCopy) === before);
    }
  },

  test_v8_object_clone_template_instance() {
    for (const [how, useFunctionTemplate] of [
      ["ObjectTemplate::NewInstance()", false],
      ["FunctionTemplate::GetFunction()->NewInstance()", true],
    ]) {
      console.log(`====== object made by ${how}`);
      const [original, copy] = nativeModule.clone_template_instance(useFunctionTemplate);
      console.log("copy is another object:", copy !== original);
      console.log("same prototype:", Object.getPrototypeOf(copy) === Object.getPrototypeOf(original));
      console.log("own keys of the copy:", Object.keys(copy).join(", "));
      console.log("copy.describeInternalFields():", copy.describeInternalFields());
      console.log("original.describeInternalFields():", original.describeInternalFields());

      copy.own = "changed on the copy";
      copy.addedToCopy = true;
      original.addedToOriginal = true;
      console.log("original after writing properties of both:", JSON.stringify(original));
      console.log("copy after writing properties of both:", JSON.stringify(copy));

      // The copy of a copy that JS gave properties of its own.
      const copyOfCopy = nativeModule.perform_object_clone(copy);
      console.log("copy of the copy:", JSON.stringify(copyOfCopy));
      console.log("same prototype:", Object.getPrototypeOf(copyOfCopy) === Object.getPrototypeOf(copy));
      console.log("copyOfCopy.describeInternalFields():", copyOfCopy.describeInternalFields());
      copyOfCopy.own = "changed on the copy of the copy";
      copyOfCopy.addedToCopyOfCopy = true;
      console.log("copy after writing properties of its copy:", JSON.stringify(copy));

      for (const [description, source] of [
        ["the original", original],
        ["the copy", copy],
        ["an object with data properties", { number: 1, string: "two" }],
      ]) {
        try {
          nativeModule.perform_object_clone_with_pending_exception(source);
          console.log(`cloning ${description} did not throw`);
        } catch (e) {
          console.log(`cloning ${description}: caught`, e.message);
        }
      }
    }
  },
});
