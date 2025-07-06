#[target_feature(enable = -1)]
//~^ ERROR expected unsuffixed literal, found `-`
//~| ERROR malformed `target_feature` attribute input
fn handler() {}

fn main() {}
