// This test case tests the incremental compilation hash (ICH) implementation
// for trait definitions.

// The general pattern followed here is: Change one thing between rev1 and rev2
// and make sure that the hash has changed, then change nothing between rev2 and
// rev3 and make sure that the hash has not changed.

// We also test the ICH for trait definitions exported in metadata. Same as
// above, we want to make sure that the change between rev1 and rev2 also
// results in a change of the ICH for the trait's metadata, and that it stays
// the same between rev2 and rev3.

//@ build-pass (FIXME(62277): could be check-pass?)
//@ revisions: cfail1 cfail2 cfail3 cfail4 cfail5 cfail6
//@ compile-flags: -Z query-dep-graph -O
//@ [cfail1]compile-flags: -Zincremental-ignore-spans
//@ [cfail2]compile-flags: -Zincremental-ignore-spans
//@ [cfail3]compile-flags: -Zincremental-ignore-spans
//@ ignore-backends: gcc

#![allow(warnings)]
#![feature(rustc_attrs)]
#![crate_type="rlib"]
#![feature(associated_type_defaults)]


// Change trait visibility
#[cfg(any(cfail1,cfail4))]
trait TraitVisibility { }

#[cfg(not(any(cfail1,cfail4)))]
#[rustc_clean(cfg="cfail2")]
#[rustc_clean(cfg="cfail3")]
#[rustc_clean(cfg="cfail5", except="opt_hir_owner_nodes,predicates_of")]
#[rustc_clean(cfg="cfail6")]
pub trait TraitVisibility { }



