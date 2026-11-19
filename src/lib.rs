pub mod ast;
pub mod compiler;
pub mod diagnostics;
pub mod driver;
pub mod lexer;
pub mod loader;
pub mod manifest;
pub mod parser;
pub mod runtime;

/// Zero-sized token signaling that an error was already reported to the user (rendered to stderr).
#[derive(Debug, Clone, Copy)]
pub struct Reported;
