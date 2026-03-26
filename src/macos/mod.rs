// This is required because the objc crate is causing a lot of warnings: https://github.com/SSheldon/rust-objc/issues/125
// Eventually we should migrate to the objc2 crate and remove this.
#![allow(unexpected_cfgs)]

mod cursor;
mod keyboard;
mod view;
mod window;

pub use window::*;
