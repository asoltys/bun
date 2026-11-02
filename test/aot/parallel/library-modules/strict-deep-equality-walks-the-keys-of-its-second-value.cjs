// util.isDeepStrictEqual and assert.deepStrictEqual walk the keys of their second value, string keys before symbol keys: which trap of a nested Proxy runs first and which of a difference and a throwing getter comes first, for object literals of the program, objects that the library made, and one of each.
const util = require("node:util");
const assert = require("node:assert");

const log = [];
const traced = (name, target) =>
  new Proxy(target, {
    get(object, key, receiver) {
      if (typeof key === "string" && key !== "constructor") log.push(name + ".get:" + key);
      return Reflect.get(object, key, receiver);
    },
  });
const thrower = () => ({
  get x() {
    throw new RangeError("x");
  },
});
const parsed = (...entries) => {
  const object = JSON.parse("{}");
  for (const [key, value] of entries) object[key] = value;
  return object;
};
function tell(label, run) {
  log.length = 0;
  let answer;
  try {
    answer = JSON.stringify(run());
  } catch (error) {
    answer = "throws " + (error && error.name);
  }
  console.log(label + ": " + answer + " | " + log.join(" "));
}
const caught = run => {
  try {
    run();
    return "passes";
  } catch (error) {
    return error.name;
  }
};

tell("literals, traps", () => util.isDeepStrictEqual({ a: traced("1a", { v: 1 }), b: traced("1b", { v: 1 }) }, { b: traced("2b", { v: 1 }), a: traced("2a", { v: 1 }) }));
tell("literals, a difference before a throw", () => util.isDeepStrictEqual({ a: thrower(), b: 1 }, { b: 2, a: thrower() }));
tell("literals, a throw before a difference", () => util.isDeepStrictEqual({ b: 2, a: thrower() }, { a: thrower(), b: 1 }));
tell("literal and parsed, traps", () => util.isDeepStrictEqual({ a: traced("1a", { v: 1 }), b: traced("1b", { v: 1 }) }, parsed(["b", traced("2b", { v: 1 })], ["a", traced("2a", { v: 1 })])));
tell("parsed and literal, traps", () => util.isDeepStrictEqual(parsed(["a", traced("1a", { v: 1 })], ["b", traced("1b", { v: 1 })]), { b: traced("2b", { v: 1 }), a: traced("2a", { v: 1 }) }));
tell("literal and parsed, a difference before a throw", () => util.isDeepStrictEqual({ a: thrower(), b: 1 }, parsed(["b", 2], ["a", thrower()])));
tell("parsed, traps", () => util.isDeepStrictEqual(parsed(["a", traced("1a", { v: 1 })], ["b", traced("1b", { v: 1 })]), parsed(["b", traced("2b", { v: 1 })], ["a", traced("2a", { v: 1 })])));
tell("parsed, a difference before a throw", () => util.isDeepStrictEqual(parsed(["a", thrower()], ["b", 1]), parsed(["b", 2], ["a", thrower()])));
tell("parsed, a throw before a difference", () => util.isDeepStrictEqual(parsed(["b", 2], ["a", thrower()]), parsed(["a", thrower()], ["b", 1])));
tell("assert.deepStrictEqual", () => caught(() => assert.deepStrictEqual({ a: thrower(), b: 1 }, { b: 2, a: thrower() })));
tell("assert.deepStrictEqual, other order", () => caught(() => assert.deepStrictEqual({ b: 2, a: thrower() }, { a: thrower(), b: 1 })));
tell("assert.notDeepStrictEqual", () => caught(() => assert.notDeepStrictEqual({ a: thrower(), b: 1 }, { b: 2, a: thrower() })));
tell("assert.notDeepStrictEqual, other order", () => caught(() => assert.notDeepStrictEqual({ b: 2, a: thrower() }, { a: thrower(), b: 1 })));
tell("without prototypes", () => util.isDeepStrictEqual({ a: thrower(), b: 1 }, { b: 2, a: thrower() }, true));
tell("without prototypes, other order", () => util.isDeepStrictEqual({ b: 2, a: thrower() }, { a: thrower(), b: 1 }, true));
tell("assert.throws with an object", () => caught(() => assert.throws(() => { throw Object.assign(new Error("m"), { info: { a: thrower(), b: 1 } }); }, { info: { b: 2, a: thrower() } })));
tell("literals with the same key order, traps", () => util.isDeepStrictEqual({ a: traced("1a", { v: 1 }), b: traced("1b", { v: 1 }) }, { a: traced("2a", { v: 1 }), b: traced("2b", { v: 1 }) }));
class Point {
  constructor(side, first, second) {
    this[first] = traced(side + first, { v: 1 });
    this[second] = traced(side + second, { v: 1 });
  }
}
tell("instances of a class, traps", () => util.isDeepStrictEqual(new Point("1", "x", "y"), new Point("2", "y", "x")));
const symbol = Symbol("s");
tell("literals with a symbol key written first, traps", () => util.isDeepStrictEqual({ [symbol]: traced("1s", { v: 1 }), a: traced("1a", { v: 1 }) }, { [symbol]: traced("2s", { v: 1 }), a: traced("2a", { v: 1 }) }));
class Tagged {
  constructor(side) {
    this[symbol] = traced(side + "s", { v: 1 });
    this.a = traced(side + "a", { v: 1 });
  }
}
tell("instances of a class with a symbol key written first, traps", () => util.isDeepStrictEqual(new Tagged("1"), new Tagged("2")));
