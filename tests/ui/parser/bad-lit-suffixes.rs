#![feature(rustc_attrs)]

#[rustc_layout_scalar_valid_range_start(0suffix)]
//~^ ERROR invalid suffix `suffix` for number literal
struct S;

fn main() {

}