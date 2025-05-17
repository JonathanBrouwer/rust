//@ build-pass (FIXME(62277): could be check-pass?)

#![allow(unused_variables)]
#![allow(dead_code)]
#![warn(unreachable_code)]

fn foo() {
    // No error here.
    let x = false && (return);
    println!("I am not dead.");
}

fn bar() {
    // But this diverges
    let x = (return) && true; //~ WARNING unreachable expression
    println!("But I am.");
}

fn main() { }
