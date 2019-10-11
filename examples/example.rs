#![cfg_attr(has_generator_trait, feature(generator_trait))]
use core::pin::Pin;
use frenetic::{Coroutine, Generator, GeneratorState, STACK_MINIMUM};

fn main() {
    // You'll need to create a stack before using Frenetic coroutines.
    let mut stack = Box::new([0u8; 8 * STACK_MINIMUM]);

    // Then, you can initialize with `Coroutine::new`.
    let mut coro = Coroutine::new(stack.as_mut(), |c| {
        let c = c.r#yield(1)?; // Yield an integer value.
        c.done("foo") // Return a string value.
    });

    // You can also interact with the yielded and returned values.
    match Pin::new(coro.as_mut()).resume() {
        GeneratorState::Yielded(1) => {}
        _ => panic!("unexpected return from resume"),
    }
    match Pin::new(coro.as_mut()).resume() {
        GeneratorState::Complete("foo") => {}
        _ => panic!("unexpected return from resume"),
    }
}
