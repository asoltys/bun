// Bun's transpiler prints the store of `var a = ..; var f; f = v;` as it is written, and not as `var f = v`. Each line has the words and the places of what is reported where the last record or the last mapped token behind such a store stands: the end of a derived constructor that calls no `super`, a pattern behind a store to a variable that a closure reads, and the call of a function that is called where it ends.
const frames = error =>
  error.stack
    .split("\n")
    .filter(line => /^\s+at /.test(line))
    .slice(0, 3)
    .map(line => line.replace(/^\s+at (?:(.*?) \()?.*?:(\d+:\d+)\)?$/, "$1@$2"))
    .join(" ");
const show = (f, ...a) => {
  try {
    console.log(f.name, JSON.stringify(f(...a)));
  } catch (error) {
    console.log(f.name, error.name, error.message, frames(error));
  }
};
const g = () => 1;
class Base {}
class EndsBehindMember extends Base {
  constructor(c) {
    var a = g();
    var f;
    f = c.x.y;
  }
}
class EndsBehindFoldedKey extends Base {
  constructor(c) {
    var a = g();
    var f;
    f = c[1 + 1].z;
  }
}
class EndsBehindCall extends Base {
  constructor(c) {
    var a = c.x;
    var f;
    f = g();
  }
}
// prettier-ignore
class EndsBehindValueOverLines extends Base {
  constructor(c) {
    var a = g();
    var f;
    f = c
      .x
      .y;
  }
}
class EndsInBlock extends Base {
  constructor(c) {
    {
      var a = g();
      var f;
      f = c.x.y;
    }
  }
}
function endsBehindMember(c) {
  return new EndsBehindMember(c);
}
function endsBehindFoldedKey(c) {
  return new EndsBehindFoldedKey(c);
}
function endsBehindCall(c) {
  return new EndsBehindCall(c);
}
function endsBehindValueOverLines(c) {
  return new EndsBehindValueOverLines(c);
}
function endsInBlock(c) {
  return new EndsInBlock(c);
}
function capturedBehindPattern(c) {
  var a = g();
  var f;
  f = c.x;
  var { b } = null;
  return () => [a, f, b];
}
function capturedNumberBehindPattern(c) {
  var a = g();
  var f;
  f = 1;
  var { b } = null;
  return () => [a, f, b];
}
// prettier-ignore
function calledWhereItEnds(c) { var r = (function () { var a = c.thrower(); var f; f = c.x; })(); return r; }
// prettier-ignore
function calledWhereItEndsInNumber(c) { var r = (function () { var a = c.thrower(); var f; f = 1; })(); return r; }
// prettier-ignore
function calledALineBelowItsStore(c) {
  var r = (function () { var a = c.thrower(); var f;
    f = c.x; })(); return r;
}
const whole = {
  x: { y: 1 },
  2: { z: 1 },
  thrower() {
    throw new Error("thrown");
  },
};
for (const f of [endsBehindMember, endsBehindFoldedKey, endsBehindCall, endsBehindValueOverLines, endsInBlock]) {
  show(f, whole);
}
show(endsBehindFoldedKey, {});
for (const f of [capturedBehindPattern, capturedNumberBehindPattern]) show(f, whole);
for (const f of [calledWhereItEnds, calledWhereItEndsInNumber, calledALineBelowItsStore]) show(f, whole);
