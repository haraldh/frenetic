// Copyright 2019 Red Hat
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Frenetic is an implementation of stackful coroutines. It is written in Rust
//! and LLVM. Notably, this approach does not require any system calls or hand-
//! crafted assembly at all.
//!
//! # Example usage
//! ```
//! # #![cfg_attr(has_generator_trait, feature(generator_trait))]
//! use frenetic::{Coroutine, Generator, GeneratorState, STACK_MINIMUM};
//! use core::pin::Pin;
//!
//! // You'll need to create a stack before using Frenetic coroutines.
//! let mut stack = [0u8; STACK_MINIMUM * 8];
//!
//! // Then, you can initialize with `Coroutine::new`.
//! let mut coro = Coroutine::new(stack.as_mut(), |c| {
//!     let c = c.r#yield(1)?; // Yield an integer value.
//!     c.done("foo") // Return a string value.
//! });
//!
//! // You can also interact with the yielded and returned values.
//! match Pin::new(coro.as_mut()).resume() {
//!     GeneratorState::Yielded(1) => {}
//!     _ => panic!("unexpected return from resume"),
//! }
//! match Pin::new(coro.as_mut()).resume() {
//!     GeneratorState::Complete("foo") => {}
//!     _ => panic!("unexpected return from resume"),
//! }
//! ```

#![cfg_attr(has_generator_trait, feature(generator_trait))]
#![deny(
    warnings,
    absolute_paths_not_starting_with_crate,
    deprecated_in_future,
    keyword_idents,
    macro_use_extern_crate,
    trivial_numeric_casts,
    unused_extern_crates,
    unused_import_braces,
    unused_qualifications,
    unused_results,
    unused_labels,
    unused_lifetimes,
    unreachable_pub,
    future_incompatible,
    missing_doc_code_examples,
    rust_2018_idioms,
    rust_2018_compatibility
)]

use core::ffi::c_void;
use core::mem::MaybeUninit;
#[cfg(has_generator_trait)]
pub use core::ops::{Generator, GeneratorState};
use core::pin::Pin;
use core::ptr;
use std::fmt::Debug;

const STACK_ALIGNMENT: usize = 16;
pub const STACK_MINIMUM: usize = 4096;

extern "C" {
    fn jump_into(into: *mut [*mut c_void; 5]) -> !;
    fn jump_swap(from: *mut [*mut c_void; 5], into: *mut [*mut c_void; 5]);
    fn jump_init(
        buff: *mut [*mut c_void; 5],
        stack: *mut u8,
        coro: *mut c_void,
        func: unsafe extern "C" fn(coro: *mut c_void) -> !,
    );
    fn stk_grows_up(c: *mut c_void) -> bool;
}

#[repr(C, align(16))]
struct Context<Y, R> {
    parent: [*mut c_void; 5],
    child: [*mut c_void; 5],
    arg: Option<Box<GeneratorState<Y, R>>>,
    canceled: bool,
}

impl<Y, R> Debug for Context<Y, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:p}: parent: {:#?} , child {:#?}",
            self as *const _, self.parent, self.child
        )?;
        Ok(())
    }
}

impl<Y, R> Default for Context<Y, R> {
    fn default() -> Self {
        Context {
            parent: [ptr::null_mut(); 5],
            child: [ptr::null_mut(); 5],
            arg: None,
            canceled: false,
        }
    }
}

#[cfg(not(has_generator_trait))]
pub trait Generator {
    /// The type of value this generator yields.
    ///
    /// This associated type corresponds to the `yield` expression and the
    /// values which are allowed to be returned each time a generator yields.
    /// For example an iterator-as-a-generator would likely have this type as
    /// `T`, the type being iterated over.
    type Yield;

    /// The type of value this generator returns.
    ///
    /// This corresponds to the type returned from a generator either with a
    /// `return` statement or implicitly as the last expression of a generator
    /// literal. For example futures would use this as `Result<T, E>` as it
    /// represents a completed future.
    type Return;

    /// Resumes the execution of this generator.
    ///
    /// This function will resume execution of the generator or start execution
    /// if it hasn't already. This call will return back into the generator's
    /// last suspension point, resuming execution from the latest `yield`. The
    /// generator will continue executing until it either yields or returns, at
    /// which point this function will return.
    ///
    /// # Return value
    ///
    /// The `GeneratorState` enum returned from this function indicates what
    /// state the generator is in upon returning. If the `Yielded` variant is
    /// returned then the generator has reached a suspension point and a value
    /// has been yielded out. Generators in this state are available for
    /// resumption at a later point.
    ///
    /// If `Complete` is returned then the generator has completely finished
    /// with the value provided. It is invalid for the generator to be resumed
    /// again.
    ///
    /// # Panics
    ///
    /// This function may panic if it is called after the `Complete` variant has
    /// been returned previously. While generator literals in the language are
    /// guaranteed to panic on resuming after `Complete`, this is not guaranteed
    /// for all implementations of the `Generator` trait.
    fn resume(self: Pin<&mut Self>) -> GeneratorState<Self::Yield, Self::Return>;
}

#[cfg(not(has_generator_trait))]
pub enum GeneratorState<Y, R> {
    /// The generator suspended with a value.
    ///
    /// This state indicates that a generator has been suspended, and typically
    /// corresponds to a `yield` statement. The value provided in this variant
    /// corresponds to the expression passed to `yield` and allows generators to
    /// provide a value each time they yield.
    Yielded(Y),

    /// The generator completed with a return value.
    ///
    /// This state indicates that a generator has finished execution with the
    /// provided value. Once a generator has returned `Complete` it is
    /// considered a programmer error to call `resume` again.
    Complete(R),
}

pub struct Finished<R>(R);

pub struct Canceled(());

pub struct Coroutine<'a, Y, R, F>
where
    F: FnMut(Control<'_, Y, R>) -> Result<Finished<R>, Canceled>,
{
    ctx: Option<Pin<Box<Context<Y, R>>>>,
    stack: &'a mut [u8],
    parent: [*mut c_void; 5],
    func: Box<F>,
}

impl<Y, R, F> Debug for Coroutine<'_, Y, R, F>
where
    F: FnMut(Control<'_, Y, R>) -> Result<Finished<R>, Canceled>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(ctx) = self.ctx.as_ref() {
            write!(f, "{:#?}", ctx)?;
        } else {
            write!(f, "None")?;
        }
        Ok(())
    }
}

unsafe extern "C" fn callback<Y, R, F>(c: *mut c_void) -> !
where
    F: FnMut(Control<'_, Y, R>) -> Result<Finished<R>, Canceled>,
{
    eprintln!(
        "callback(): c {:#?}\n",
        *(c as *const Coroutine<'_, Y, R, F>)
    );

    // Cast the incoming pointers to their correct types.
    // See `Coroutine::new()`.
    let coro = c as *mut Coroutine<'_, Y, R, F>;

    // Yield control to the parent. The first call to `Generator::resume()`
    // will resume at this location. The `Coroutine::new()` function is
    // responsible to move the closure into this stack while we are yielded.

    eprintln!(
        "callback(): before jump_swap {:#?}\np: {:#?}\n",
        (*coro).ctx.as_ref(),
        (*coro).parent,
    );
    jump_swap(
        (*coro).ctx.as_mut().unwrap().child.as_mut_ptr() as _,
        (*coro).parent.as_mut_ptr() as _,
    );
    eprintln!(
        "callback(): after jump_swap {:#?}\np: {:#?}\n",
        (*coro).ctx.as_ref(),
        (*coro).parent
    );

    let fnc = &mut *(*coro).func;

    // Call the closure. If the closure returns, then move the return value
    // into the argument variable in `Generator::resume()`.
    if let Ok(r) = (fnc)(Control(&mut (*coro).ctx.as_mut().unwrap())) {
        let _ = (*coro)
            .ctx
            .as_mut()
            .unwrap()
            .arg
            .replace(Box::new(GeneratorState::Complete(r.0)));
    }

    // We cannot be resumed, so jump away forever.
    jump_into((*coro).ctx.as_mut().unwrap().parent.as_mut_ptr() as _);
}

impl<'a, Y, R, F> Coroutine<'a, Y, R, F>
where
    F: FnMut(Control<'_, Y, R>) -> Result<Finished<R>, Canceled>,
{
    /// Spawns a new coroutine.
    ///
    /// This sets up the stack, and executes the closure within that stack.
    ///
    /// # Arguments
    ///
    /// * `stack` - A stack for this coroutine to use.
    /// This must be larger than `STACK_MINIMUM`, currently 4096, or Frenetic
    /// will panic.
    /// NOTE: It is up to the caller to properly allocate this stack. We
    /// recommend the stack include a guard page.
    ///
    /// * `func` - The closure to be executed as part of the coroutine.
    pub fn new(stack: &'a mut [u8], func: F) -> Box<Self> {
        assert!(stack.len() >= STACK_MINIMUM);

        // These variables are going to receive output from the callback
        // function above. Specifically, the callback function is going to
        // allocate space for a Context and our closure on the new stack. Then,
        // it is going to store references to those instances inside these
        // variables.
        let mut cor = Box::new(Coroutine {
            ctx: Some(Box::pin(Context::<Y, R>::default())),
            stack: stack,
            func: Box::new(func),
            parent: [ptr::null_mut(); 5],
        });

        let mut test_ptr = MaybeUninit::<bool>::uninit();

        unsafe {
            // Calculate the aligned top of the stack.
            let top = if stk_grows_up(test_ptr.as_mut_ptr() as _) {
                let top = cor.stack.as_mut_ptr();
                top.add(top.align_offset(STACK_ALIGNMENT))
            } else {
                let top = cor.stack.as_mut_ptr().add(cor.stack.len() - 1);
                if top.align_offset(STACK_ALIGNMENT) != 0 {
                    let top = top.sub(STACK_ALIGNMENT);
                    top.add(top.align_offset(STACK_ALIGNMENT))
                } else {
                    top
                }
            };

            eprintln!("Stack {:p} - {:p}\n", cor.stack.as_mut_ptr(), top);

            let mut buff: [*mut c_void; 5] = [ptr::null_mut(); 5];
            eprintln!("new(): before jump_init cor {:#?}\n", &mut cor,);
            eprintln!(
                "new(): before jump_init {:#?}\np: {:#?}\n",
                cor.ctx.as_ref(),
                buff.as_mut_ptr()
            );
            // Call into the callback on the specified stack.
            jump_init(
                cor.parent.as_mut_ptr() as _,
                top,
                cor.as_mut() as *mut _ as _,
                callback::<Y, R, F>,
            );
            eprintln!("new(): after jump_init {:?}\n", cor);
        }

        cor
    }
}

pub struct Control<'a, Y, R>(&'a mut Context<Y, R>);

impl<'a, Y, R> Control<'a, Y, R> {
    /// Pauses execution of this coroutine, saves function position, and passes
    /// control back to parent.
    /// Returns a `Canceled` error if the parent has been dropped.
    ///
    /// # Arguments
    ///
    /// * `arg` - Passed on to the argument variable for the generator, if it
    /// exists.
    pub fn r#yield(self, arg: Y) -> Result<Self, Canceled> {
        if self.0.canceled {
            return Err(Canceled(()));
        }

        self.0.arg = Some(Box::new(GeneratorState::Yielded(arg)));

        unsafe {
            eprintln!("yield(): before jump_swap {:#?}\n", self.0);
            // Save our current position and yield control to the parent.
            jump_swap(
                self.0.child.as_mut_ptr() as _,
                self.0.parent.as_mut_ptr() as _,
            );
            eprintln!("yield(): after jump_swap {:#?}\n", self.0);

            if (&mut self.0.canceled as *mut bool).read_volatile() {
                return Err(Canceled(()));
            }
        }

        if self.0.canceled {
            return Err(Canceled(()));
        }

        Ok(self)
    }

    /// Finishes execution of this coroutine.
    pub fn done<E>(self, arg: R) -> Result<Finished<R>, E> {
        Ok(Finished(arg))
    }
}

impl<'a, Y, R, F> Generator for Coroutine<'a, Y, R, F>
where
    F: FnMut(Control<'_, Y, R>) -> Result<Finished<R>, Canceled>,
{
    type Yield = Y;
    type Return = R;

    /// Resumes a paused coroutine.
    /// Re-initialize stack and continue execution where it was left off.
    fn resume(mut self: Pin<&mut Self>) -> GeneratorState<Y, R> {
        match self.ctx {
            None => panic!("Called Generator::resume() after completion!"),
            Some(ref mut p) => unsafe {
                p.arg = None;
                eprintln!("resume(): before jump_swap {:#?}\n", p);
                // Jump back into the child.
                jump_swap(p.parent.as_mut_ptr() as _, p.child.as_mut_ptr() as _);
                eprintln!("resume(): after jump_swap {:#?}\n", p);
            },
        }

        // Clear the pointer as the value is about to become invalid.
        let state = *(self.ctx.as_mut().unwrap().arg.take().unwrap());

        // If the child coroutine has completed, we are done. Make it so that
        // we can never resume the coroutine by clearing the reference.
        if let GeneratorState::Complete(_) = state {
            self.ctx.as_mut().unwrap().canceled = true;
            let _old = self.ctx.take();
        }

        state
    }
}

impl<'a, Y, R, F> Drop for Coroutine<'a, Y, R, F>
where
    F: FnMut(Control<'_, Y, R>) -> Result<Finished<R>, Canceled>,
{
    fn drop(&mut self) {
        // If we are still able to resume the coroutine, do so.
        if let Some(ref mut x) = self.ctx {
            unsafe {
                // set the argument pointer to null, `Control::r#yield()` will return `Canceled`.
                x.canceled = true;
                jump_swap(x.parent.as_mut_ptr() as _, x.child.as_mut_ptr() as _);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack() {
        let mut stack = [1u8; STACK_MINIMUM * 4];

        let mut coro = Coroutine::new(stack.as_mut(), |c| {
            let c = c.r#yield(1)?;
            c.done("foo")
        });

        match Pin::new(coro.as_mut()).resume() {
            GeneratorState::Yielded(1) => {}
            _ => panic!("unexpected return from resume"),
        }

        match Pin::new(coro.as_mut()).resume() {
            GeneratorState::Complete("foo") => {}
            _ => panic!("unexpected return from resume"),
        }
    }

    #[test]
    fn heap() {
        let mut stack = Box::new([1u8; STACK_MINIMUM]);

        let mut coro = Coroutine::new(stack.as_mut(), |c| {
            let c = c.r#yield(1)?;
            c.done("foo")
        });

        match Pin::new(coro.as_mut()).resume() {
            GeneratorState::Yielded(1) => {}
            _ => panic!("unexpected return from resume"),
        }

        match Pin::new(coro.as_mut()).resume() {
            GeneratorState::Complete("foo") => {}
            _ => panic!("unexpected return from resume"),
        }
    }

    #[test]
    fn cancel() {
        let mut cancelled = false;

        {
            let mut stack = [1u8; STACK_MINIMUM];

            let mut coro = Coroutine::new(stack.as_mut(), |c| match c.r#yield(1) {
                Ok(c) => c.done("foo"),
                Err(v) => {
                    cancelled = true;
                    Err(v)
                }
            });

            match Pin::new(coro.as_mut()).resume() {
                GeneratorState::Yielded(1) => {}
                _ => panic!("unexpected return from resume"),
            }

            // Coroutine is cancelled when it goes out of scope.
        }

        assert!(cancelled);
    }

    #[test]
    fn coro_early_drop_yield_done() {
        let mut stack = [1u8; STACK_MINIMUM];

        let _coro = Coroutine::new(stack.as_mut(), |c| {
            let c = c.r#yield(1)?;
            c.done("foo")
        });
    }

    #[test]
    fn coro_early_drop_done_only() {
        let mut stack = [1u8; STACK_MINIMUM];

        let _coro = Coroutine::new(stack.as_mut(), |c: Control<'_, i32, &str>| c.done("foo"));
    }

    #[test]
    fn coro_early_drop_result_ok() {
        let mut stack = [1u8; STACK_MINIMUM];

        let _coro = Coroutine::new(stack.as_mut(), |_c: Control<'_, i32, &str>| {
            Ok(Finished("foo"))
        });
    }

    #[test]
    fn coro_early_drop_result_err() {
        let mut stack = [1u8; STACK_MINIMUM];

        let _coro = Coroutine::new(stack.as_mut(), |_c: Control<'_, i32, &str>| {
            Err(Canceled(()))
        });
    }

    #[test]
    #[should_panic(expected = "stack.len() >= STACK_MINIMUM")]
    fn small_stack() {
        let mut stack = [1u8; STACK_MINIMUM - 1];
        let _coro = Coroutine::new(stack.as_mut(), |_c: Control<'_, i32, &str>| {
            Err(Canceled(()))
        });
    }
}
