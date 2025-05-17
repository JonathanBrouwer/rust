#![allow(unused_variables)]
#![allow(unused_assignments)]
#![allow(dead_code)]
#![deny(unreachable_code)]
#![feature(never_type, type_ascription)]

fn a() {
    // the cast is unreachable:
    // but we don't lint because this fails to typecheck
    let x = {return} as !;
    //~^ ERROR non-primitive cast
}

fn main() { }
