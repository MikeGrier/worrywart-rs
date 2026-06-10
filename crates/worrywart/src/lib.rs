// Copyright (c) 2026 Michael Grier

//! A library that worries, maybe too much, about child processes
//!
//! This is the core library crate for `worrywart-rs`.

/// Returns a hello-world greeting from this crate.
pub fn hello() -> String {
    format!("Hello from {}!", "worrywart")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_greets_this_crate() {
        assert!(hello().contains("worrywart"));
    }
}
