// This file is part of libfringe, a low-level green threading library.
// Copyright (c) whitequark <whitequark@whitequark.org>
// Licensed under the Apache License, Version 2.0, <LICENSE-APACHE or
// http://apache.org/licenses/LICENSE-2.0> or the MIT license <LICENSE-MIT or
// http://opensource.org/licenses/MIT>, at your option. This file may not be
// copied, modified, or distributed except according to those terms.

//! Generators.
//!
//! Generators allow repeatedly suspending the execution of a function,
//! returning a value to the caller, and resuming the suspended function
//! afterwards.

use core::marker::PhantomData;
use core::{ptr, mem};
use core::cell::Cell;

use stack;
use debug;
use stack_pointer::StackPointer;

#[derive(Debug, Clone, Copy)]
pub enum State {
  /// Generator can be resumed. This is the initial state.
  Runnable,
  /// Generator cannot be resumed. This is the state of the generator after
  /// the generator function has returned or panicked.
  Unavailable
}

/// Generator wraps a function and allows suspending its execution more than once, returning
/// a value each time.
///
/// The first time `resume(input0)` is called, the function is called as `f(yielder, input0)`.
/// It runs until it suspends its execution through `yielder.suspend(output0)`, after which
/// `resume(input0)` returns `output0`. The function can be resumed again using `resume(input1)`,
/// after which `yielder.suspend(output0)` returns `input1`, and so on. Once the function returns,
/// the `resume()` call will return `None`, and it will return `None` every time it is called
/// after that.
///
/// If the generator function panics, the panic is propagated through the `resume()` call as usual.
///
/// After the generator function returns or panics, it is safe to reclaim the generator stack
/// using `unwrap()`.
///
/// `state()` can be used to determine whether the generator function has returned;
/// the state is `State::Runnable` after creation and suspension, and `State::Unavailable`
/// once the generator function returns or panics.
///
/// When the input type is `()`, a generator implements the Iterator trait.
///
/// # Example
///
/// ```
/// use fringe::{OsStack, Generator};
///
/// let stack = OsStack::new(0).unwrap();
/// let mut add_one = Generator::new(stack, move |yielder, mut input| {
///   loop {
///     if input == 0 { break }
///     input = yielder.suspend(input + 1)
///   }
/// });
/// println!("{:?}", add_one.resume(2)); // prints Some(3)
/// println!("{:?}", add_one.resume(3)); // prints Some(4)
/// println!("{:?}", add_one.resume(0)); // prints None
/// ```
///
/// # Iterator example
///
/// ```
/// use fringe::{OsStack, Generator};
///
/// let stack = OsStack::new(0).unwrap();
/// let mut nat = Generator::new(stack, move |yielder, ()| {
///   for i in 1.. { yielder.suspend(i) }
/// });
/// println!("{:?}", nat.next()); // prints Some(0)
/// println!("{:?}", nat.next()); // prints Some(1)
/// println!("{:?}", nat.next()); // prints Some(2)
/// ```
#[derive(Debug)]
pub struct Generator<Input: Send, Output: Send, Stack: stack::Stack> {
  state:     State,
  stack:     Stack,
  stack_id:  debug::StackId,
  stack_ptr: StackPointer,
  phantom:   PhantomData<(*const Input, *const Output)>
}

impl<Input, Output, Stack> Generator<Input, Output, Stack>
    where Input: Send, Output: Send, Stack: stack::Stack {
  /// Creates a new generator.
  ///
  /// See also the [contract](../trait.GuardedStack.html) that needs to be fulfilled by `stack`.
  pub fn new<F>(stack: Stack, f: F) -> Generator<Input, Output, Stack>
      where Stack: stack::GuardedStack,
            F: FnOnce(&mut Yielder<Input, Output>, Input) + Send {
    unsafe { Generator::unsafe_new(stack, f) }
  }

  /// Same as `new`, but does not require `stack` to have a guard page.
  ///
  /// This function is unsafe because the generator function can easily violate
  /// memory safety by overflowing the stack. It is useful in environments where
  /// guarded stacks do not exist, e.g. in absence of an MMU.
  ///
  /// See also the [contract](../trait.Stack.html) that needs to be fulfilled by `stack`.
  pub unsafe fn unsafe_new<F>(stack: Stack, f: F) -> Generator<Input, Output, Stack>
      where F: FnOnce(&mut Yielder<Input, Output>, Input) + Send {
    unsafe extern "C" fn generator_wrapper<Input, Output, Stack, F>(env: usize, stack_ptr: StackPointer) -> !
        where Input: Send, Output: Send, Stack: stack::Stack,
              F: FnOnce(&mut Yielder<Input, Output>, Input) {
      // Retrieve our environment from the callee and return control to it.
      let f = ptr::read(env as *const F);
      let (data, stack_ptr) = StackPointer::swap(0, stack_ptr, None);
      // See the second half of Yielder::suspend_bare.
      let input = ptr::read(data as *const Input);
      // Run the body of the generator.
      let mut yielder = Yielder::new(stack_ptr);
      f(&mut yielder, input);
      // Past this point, the generator has dropped everything it has held.
      loop { yielder.suspend_bare(None); }
    }

    let stack_id  = debug::StackId::register(&stack);
    let stack_ptr = StackPointer::init(&stack, generator_wrapper::<Input, Output, Stack, F>);

    // Transfer environment to the callee.
    let stack_ptr = StackPointer::swap(&f as *const F as usize, stack_ptr, Some(&stack)).1;
    mem::forget(f);

    Generator {
      state:     State::Runnable,
      stack:     stack,
      stack_id:  stack_id,
      stack_ptr: stack_ptr,
      phantom:   PhantomData
    }
  }

  /// Resumes the generator and return the next value it yields.
  /// If the generator function has returned, returns `None`.
  #[inline]
  pub fn resume(&mut self, input: Input) -> Option<Output> {
    match self.state {
      State::Runnable => {
        // Set the state to Unavailable. Since we have exclusive access to the generator,
        // the only case where this matters is the generator function panics, after which
        // it must not be invocable again.
        self.state = State::Unavailable;

        // Switch to the generator function, and retrieve the yielded value.
        let val = unsafe {
          let (data_out, stack_ptr) = StackPointer::swap(&input as *const Input as usize, self.stack_ptr, Some(&self.stack));
          self.stack_ptr = stack_ptr;
          mem::forget(input);
          ptr::read(data_out as *const Option<Output>)
        };

        // Unless the generator function has returned, it can be switched to again, so
        // set the state to Runnable.
        if val.is_some() { self.state = State::Runnable }

        val
      }
      State::Unavailable => None
    }
  }

  /// Returns the state of the generator.
  #[inline]
  pub fn state(&self) -> State { self.state }

  /// Extracts the stack from a generator when the generator function has returned.
  /// If the generator function has not returned
  /// (i.e. `self.state() == State::Runnable`), panics.
  pub fn unwrap(self) -> Stack {
    match self.state {
      State::Runnable    => panic!("Argh! Bastard! Don't touch that!"),
      State::Unavailable => self.stack
    }
  }
}

/// Yielder is an interface provided to every generator through which it
/// returns a value.
#[derive(Debug)]
pub struct Yielder<Input: Send, Output: Send> {
  stack_ptr: Cell<StackPointer>,
  phantom: PhantomData<(*const Input, *const Output)>
}

impl<Input, Output> Yielder<Input, Output>
    where Input: Send, Output: Send {
  fn new(stack_ptr: StackPointer) -> Yielder<Input, Output> {
    Yielder {
      stack_ptr: Cell::new(stack_ptr),
      phantom: PhantomData
    }
  }

  #[inline(always)]
  fn suspend_bare(&self, val: Option<Output>) -> Input {
    unsafe {
      let (data, stack_ptr) = StackPointer::swap(&val as *const Option<Output> as usize, self.stack_ptr.get(), None);
      self.stack_ptr.set(stack_ptr);
      mem::forget(val);
      ptr::read(data as *const Input)
    }
  }

  /// Suspends the generator and returns `Some(item)` from the `resume()`
  /// invocation that resumed the generator.
  #[inline(always)]
  pub fn suspend(&self, item: Output) -> Input {
    self.suspend_bare(Some(item))
  }
}

impl<Output, Stack> Iterator for Generator<(), Output, Stack>
    where Output: Send, Stack: stack::Stack {
  type Item = Output;

  fn next(&mut self) -> Option<Self::Item> { self.resume(()) }
}
