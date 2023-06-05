use std::{
    io,
    process::{self, Child, Command},
};

pub mod python;
pub mod rust;

#[derive(PartialEq, Debug)]
pub enum BuildResult {
    Success,
    Failure,
}
#[derive(Debug)]
pub enum RunResult {
    Success,
    Failure,
}

pub trait Language {
    fn build(&self) -> BuildResult;
    fn run(
        &self,
        configure: fn(command: &mut Command) -> &mut Command,
    ) -> io::Result<process::Child>;
}

pub fn detect_language(name: &str) -> Box<dyn Language> {
    match name {
        "python" => Box::new(python::Python {}),
        "rust" => Box::new(rust::Rust {}),
        _ => panic!("Language not supported"),
    }
}
