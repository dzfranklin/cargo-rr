# cargo-rr

![Crates.io](https://img.shields.io/crates/v/cargo-rr)
![MIT Licensed](https://img.shields.io/crates/l/cargo-rr)

A light wrapper around [`rr`](https://rr-project.org/), the time-travelling debugger.

> Do you find yourself running the same test over and over in the debugger,
> trying to figure out how the code got in a bad state? We have a tool for you!
> Easy to install and setup, it will record an execution trace, and that gives
> magical new powers to gdb. Step backwards, run backwards, see where variables
> changed their value or when a function was last called on an object (using
> conditional breakpoints). [(source)][about-quote-source]


## Example
Suppose we ran a test `my_test` and got a failure. We first re-run the test under `rr`
to record the entire execution (including everything else on your system the test
interacts with).

```bash
> cargo rr test my_test

thread 'main' panicked at 'assertion failed: `(left == right)`
  left: `1`,
 right: `2`', tests/tests.rs:100

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.06s
```

Let's replay the recording.

```bash
> cargo rr replay

(rr) continue

thread 'main' panicked at 'assertion failed: `(left == right)`
  left: `1`,
 right: `42`', tests/tests.rs:100

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.06s
```

We'll go backwards until we return to the failed assertion so we can have a look at `a` and `b`.

```bash
(rr) break tests/tests.rs:100
(rr) reverse-continue
Continuing.

Breakpoint 1, tests::main () at tests/tests.rs:100
100         assert_eq!(a, b);

(rr) print a
$1 = 1

(rr) print b
$2 = 42
```

I wonder where `a` is set to `1`?

```bash
(rr) watch -l a
(rr) reverse-continue

Old value = 1
New value = -1992923200
0x000055dcac485ed9 in tests::main () at tests/tests.rs:30
30          let a = some_calculation();

```

Note that since we're going in reverse the old and new values are backwards.
`-1992923200` is whatever used to be at the address `0x000055dcac485ed9`
before it was used for `a`.

## Installation

`cargo-rr` is a [custom cargo subcommand](custom-cargo-subcommands). It can be installed
by installing the `cargo-rr` package from `crates.io`: just run
```bash
cargo install cargo-rr
```
in a terminal. After installing you can run `cargo rr` in the terminal to access `cargo-rr`.

## Usage

Run `cargo rr test` or `cargo rr run` with any options you'd normally give to `cargo test` or `cargo run`. For example, you might run `cargo rr test --test my_integration_test some_filter`.

Once you've made a recording you can replay the last recording in a debugger with `cargo rr replay`.

### Advanced

You can pass options to rr by putting them after the delimiter `--`. If you do so, you are responsible for making sure the options you set don't conflict with the options `cargo-rr` sets for you. In general, you should be fine if you avoid customizing where traces go or what they're named. Please file a bug if you run into a conflict so I can add the option you want to `cargo-rr`.

`cargo rr test` and `cargo rr run` call `rr record` under the hood, so you can see the full list of rr options by running `rr record -h`. For example, `cargo rr test --test my_test -- --chaos` runs the tests under chaos mode, which randomizes scheduling decisions to try to surface concurrency bugs.

`cargo rr replay` calls `rr replay` under the hood, so you can see the full list of rr options by running `rr replay -h`. For example, `rr replay -- --stats=10000` displays brief stats every 10,000 steps.

Run `cargo rr help` to see the full usage.

[about-quote-source]: https://developer.chrome.com/blog/chromium-chronicle-13/
[crates-io]: https://crates.io/crates/cargo-rr
[custom-cargo-subcommands]: https://doc.rust-lang.org/stable/book/ch14-05-extending-cargo.html
