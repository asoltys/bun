// @bun
// The same program as stack-positions-of-tokens-that-start-a-line.mjs from the third line on. `bun build --target=bun` writes the first line: such a file runs as it is written and has no source map, so a stack trace shows each place as JavaScriptCore records it. It is ASCII on purpose: `bun <file>` reads such a file as Latin-1.
function fail() {
  throw new Error("fail");
}
function quiet() {}
function getFail() {
  return fail;
}
const holder = {
  nothing: undefined,
  fail,
  quiet,
  self() {
    return this;
  },
  get getter() {
    return fail();
  },
  tag() {
    return fail();
  },
};
const use = (...values) => values.length;
const key = () => "getter";
const iterable = {
  [Symbol.iterator]() {
    return fail();
  },
};
const inThisFile = /stack-positions-of-tokens-that-start-a-line/;
function show(label, run) {
  try {
    run();
    console.log(label, "| nothing is thrown");
  } catch (error) {
    const lines = String(error.stack)
      .split("\n")
      .filter(line => inThisFile.test(line));
    console.log(label, "|", error.name, "|", lines.map(line => /:(\d+:\d+)\)?$/.exec(line)[1]).join(" "));
  }
}
// prettier-ignore
{
show("a member of undefined", () => {
  use(
    holder
      .nothing
      .member,
  );
});
show("a member of undefined behind ?.", () => {
  use(
    holder
      ?.nothing
      .member,
  );
});
show("an element of undefined", () => {
  use(
    holder
      .nothing
      [0],
  );
});
show("a member in quotes", () => {
  use(
    holder
      .nothing
      ["a-b"],
  );
});
show("a store to a member of undefined", () => {
  holder
    .nothing
    .member = 1;
});
show("a getter behind a call", () => {
  use(
    holder.self()
      .getter,
  );
});
show("a getter behind a call and ?.", () => {
  use(
    holder.self()
      ?.getter,
  );
});
show("an element behind a call and ?.", () => {
  use(
    holder.self()
      ?.[key()],
  );
});
show("a getter behind a name", () => {
  use(
    holder
      .getter,
  );
});
show("a getter behind a name and ?.", () => {
  use(
    holder
      ?.getter,
  );
});
show("an element behind a name and ?.", () => {
  use(
    holder
      ?.[key()],
  );
});
show("a method", () => {
  use(
    holder
      .self()
      .fail(),
  );
});
show("a method that is undefined", () => {
  use(
    holder
      .nothing(),
  );
});
show("a method with its arguments on the next line", () => {
  use(
    holder.nothing
      (1),
  );
});
show("an optional call", () => {
  use(
    holder.fail
      ?.(),
  );
});
show("a function that is called where it is written", () => {
  (() => {
    fail();
  })();
});
show("a function that is called where it is written, with arguments", () => {
  (function (first) {
    fail();
  })(
    1,
  );
});
show("a call of what a call returns", () => {
  use(
    holder.self
      .bind(holder)()
      .nothing
      (),
  );
});
show("a callee in parentheses", () => {
  (holder.fail)
    ();
});
show("an element that is called", () => {
  holder["fail"]
    ();
});
show("a function that is used once", () => {
  const once = () => {
    fail();
  };
  once();
});
show("what a call returns, held by a constant that is used once", () => {
  const made = getFail();
  const result = made();
  use(result);
});
show("a loop", () => {
  for (const value
    of iterable) use(value);
});
show("a loop over a list that takes values", () => {
  for (const [
    value,
  ] of iterable) use(value);
});
show("a loop behind an empty list that starts a line", () => {
  for (const
    [] of iterable) use(0);
});
show("a loop behind a list with a hole that starts a line", () => {
  for (const
    [,] of iterable) use(0);
});
show("a loop behind an empty list in an object that takes values", () => {
  for (const { a:
    [] } of iterable) use(0);
});
show("a list that takes values", () => {
  const
    [first] = holder.nothing;
  use(first);
});
show("a list in a list that takes values", () => {
  const [
    [first],
  ] = [holder.nothing];
  use(first);
});
show("a member behind a backquote that starts a line", () => {
  use(
    holder.quiet
      `text`.member,
  );
});
show("an element behind a backquote that starts a line", () => {
  use(
    holder.quiet
      `text`[0],
  );
});
show("a call behind a backquote that starts a line", () => {
  use(
    holder.quiet
      `text`(),
  );
});
show("an optional call behind a backquote that starts a line", () => {
  use(
    holder.self
      `text`?.(),
  );
});
show("a tagged template behind a backquote that starts a line", () => {
  use(
    holder.quiet
      `text``more`,
  );
});
show("a member behind a tagged template of a name", () => {
  use(
    quiet
      `text`.member,
  );
});
show("an element behind a tagged template of a name", () => {
  use(
    quiet
      `text`[0],
  );
});
show("a call behind a tagged template of a name", () => {
  use(
    quiet
      `text`(),
  );
});
show("an optional call behind a tagged template of a name", () => {
  use(
    getFail
      `text`?.(),
  );
});
show("a tagged template behind a tagged template of a name", () => {
  use(
    getFail
      `text``more`,
  );
});
show("a tagged template", () => {
  use(
    holder.tag
      `text`,
  );
});
show("a tagged template of a name", () => {
  const tag = holder.tag;
  use(
    tag
      `text`,
  );
});
show("an element that is called, with a key that is used once", () => {
  let name = "fail";
  holder[name](1);
});
show("a member behind a call of a function that its block declares below", () => {
  {
    use(
      make()
        .member,
    );
    function make() {}
  }
});
show("the this of the module", () => {
  (this
  )
    (1);
});
show("an optional call of eval", () => {
  use(
    eval
      ?.("null.member"),
  );
});
show("an optional call with an equality that begins with a text", () => {
  use(
    key()
      ?.("a" === holder.nothing),
  );
});
show("an element by a member of a constant that is used once", () => {
  const once = holder.self();
  return null
    [once.nothing];
});
show("an element by a comparison of a text with typeof", () => {
  use(
    quiet()
      ["undefined" == typeof holder],
  );
});
show("an element by an equality that begins with void", () => {
  use(
    quiet()
      [void 0 === holder],
  );
});
show("a call with an equality that begins with undefined", () => {
  (0, holder.nothing
  )
    (undefined !== holder);
});
show("an optional call with an object that new makes", () => {
  use(
    key()
      ?.(new Object({ a: holder })),
  );
});
show("a constant that is used once, called behind a comma", () => {
  const once = getFail();
  (0, once)();
});
show("a constant that is used once, called where a condition that is known chooses it", () => {
  const once = getFail();
  (true ? once : quiet)();
});
show("a constant that is used once, called as the only element of a list", () => {
  const once = { a: 1 };
  ([once][0])();
});
show("a member that is called where a condition that is known chooses it", () => {
  (true ? holder.nothing : 0
  )
    (1);
});
}
