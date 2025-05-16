//@ check-pass

#![allow(deprecated, invalid_value)]
#![deny(unreachable_code)]
fn infallible() -> std::convert::Infallible {
    loop {}
}

fn main() {
    if false {
        infallible();
    }

    let _x = 1;
    panic!()
}