// This file is part of libfringe, a low-level green threading library.
// Copyright (c) Nathan Zadoks <nathan@nathan7.eu>
// See the LICENSE file included in this distribution.
#![feature(asm)]
#![feature(slice_patterns)]
#![feature(unboxed_closures)]
#![feature(unique)]
#![feature(pub_restricted)]
#![feature(specialization)]
#![cfg_attr(target_arch = "x86", feature(naked_functions, core_intrinsics))]
#![no_std]

//! libfringe is a library implementing lightweight context switches,
//! without relying on kernel services. It can be used in hosted environments
//! (using `std`) as well as on bare metal (using `core`).
//!
//! It provides high-level, safe abstractions:
//!
//!   * an implementation of internal iterators, also known as generators,
//!     [Generator](generator/struct.Generator.html).
//!
//! It also provides low-level, *very* unsafe building blocks:
//!
//!   * a flexible, low-level context-swapping mechanism,
//!     [Context](struct.Context.html);
//!   * a trait that can be implemented by stack allocators,
//!     [Stack](struct.Stack.html);
//!   * a stack allocator based on anonymous memory mappings with guard pages,
//!     [OsStack](struct.OsStack.html).
//!
//! **FIXME:** not actually safe yet in presence of unwinding

#[cfg(test)]
#[macro_use]
extern crate std;

#[macro_use]
extern crate log;
extern crate void;

pub use stack::Stack;
pub use stack::GuardedStack;
pub use stack_pointer::StackPointer;
pub use context::Context;
pub use generator::Generator;

#[cfg(any(unix, windows))]
pub use os::Stack as OsStack;

mod arch;
mod debug;

mod stack;
mod stack_pointer;
mod context;
mod fat_args;
pub mod generator;
pub mod session;

#[cfg(any(unix, windows))]
mod os;
