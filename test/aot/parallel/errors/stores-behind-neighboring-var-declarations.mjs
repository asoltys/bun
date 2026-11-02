// Bun's transpiler prints `var g; g = value;` as `var g = value` only where `var g;` does not stand behind another `var` declaration. Behind one the store is printed as it is written: each line has the words and the places of what is thrown in the store or behind it.
const place = line => (/:(\d+:\d+)\)?$/.exec(line) ?? [, "?"])[1];
const frames = error => String(error.stack).split("\n").slice(1, 3).map(place).join(" < ");
const show = (name, run) => { try { run(); console.log(name, "| no error"); } catch (error) { console.log(name, "|", String(error.message), "|", frames(error)); } };
const one = () => 1;
const same = value => value;
const nothing = () => undefined;
show("a pattern behind the store", () => { var a = one(); var g; g = same(a); const { x } = null; return [g, x]; });
show("a pattern behind the store, a statement a line", () => {
  var a = one();
  var g;
  g = same(a);
  const { x } = null;
  return [g, x];
});
show("a list behind the store", () => { var a = one(); var g; g = same(a); var [x] = 5; return [g, x]; });
show("a list behind the store, a statement a line", () => {
  var a = one();
  var g;
  g = same(a);
  const [y] = null;
  return [g, y];
});
show("a folded sum as the key", () => { var a = nothing(); var g; g = a[1 + 1].c; return g; });
show("a sum of texts as the key", () => { var a = nothing(); var g; g = a["x" + "y"].z; return g; });
show("a constant as the key", () => { const k = "key"; var a = nothing(); var g; g = a[k]; return g; });
show("keys in brackets", () => { var a = one(); var g; g = a["b"]["c"]; return g; });
show("a call of nothing", () => { var a = nothing(); var g; g = a("x" + "y", 1 + 2); return g; });
show("typeof compared", () => { var a = nothing(); var g; g = typeof a === "undefined" ? a.x.y : 0; return g; });
show("compared the other way", () => { var a = nothing(); var g; g = "undefined" === typeof a ? a.x : 0; return g; });
show("a member of nothing", () => { var a = nothing(); var g; g = a.b; return g; });
show("a value over two lines", () => {
  var a = nothing();
  var g;
  g =
    a["key"].c;
  return g;
});
show("behind two declarations", () => { var a = nothing(); var b = one(); var g; g = a[b + 1 + 1].c; return g; });
show("behind a declaration of two names", () => { var a = nothing(), b = one(); var g; g = a[1 + 1].c; return [b, g]; });
show("behind a declaration without a value", () => { var a; var g; g = a[1 + 1].c; return g; });
show("in a block", () => { { var a = nothing(); var g; g = a[1 + 1].c; } return g; });
show("in a loop", () => { for (;;) { var a = nothing(); var g; g = a[1 + 1].c; return g; } });
show("in a case", () => { switch (one()) { case 1: var a = nothing(); var g; g = a[1 + 1].c; return g; } });
show("in a try block", () => { try { var a = nothing(); var g; g = a[1 + 1].c; return g; } finally { same(a); } });
show("in a function declaration", () => { function inner() { var a = nothing(); var g; g = a[1 + 1].c; return g; } return inner(); });
show("in a method", () => ({ method() { var a = nothing(); var g; g = a[1 + 1].c; return g; } }).method());
show("in a generator", () => { function* steps() { var a = nothing(); var g; g = a[1 + 1].c; yield g; } return steps().next(); });
show("in a constructor", () => new (class { constructor() { var a = nothing(); var g; g = a[1 + 1].c; this.g = g; } })());
show("in a static block", () => class { static { var a = nothing(); var g; g = a[1 + 1].c; same(g); } });
await (async () => { var a = nothing(); var g; g = a[1 + 1].c; return g; })().catch(error => console.log("in an async function |", String(error.message), "|", frames(error)));
