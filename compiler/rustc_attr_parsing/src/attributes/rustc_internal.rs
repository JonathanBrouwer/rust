use rustc_attr_data_structures::AttributeKind;
use rustc_feature::{AttributeTemplate, template};
use rustc_span::sym;

use crate::attributes::{AttributeOrder, OnDuplicate, SingleAttributeParser};
use crate::context::{AcceptContext, Stage};
use crate::parser::ArgParser;

pub(crate) struct RustcObjectLifetimeDefaultParser;

impl<S: Stage> SingleAttributeParser<S> for RustcObjectLifetimeDefaultParser {
    const PATH: &[rustc_span::Symbol] = &[sym::rustc_object_lifetime_default];
    const ATTRIBUTE_ORDER: AttributeOrder = AttributeOrder::KeepFirst;
    const ON_DUPLICATE: OnDuplicate<S> = OnDuplicate::Error;
    const TEMPLATE: AttributeTemplate = template!(Word);

    fn convert(cx: &mut AcceptContext<'_, '_, S>, args: &ArgParser<'_>) -> Option<AttributeKind> {
        if let Err(span) = args.no_args() {
            cx.expected_no_args(span);
            return None;
        }

        Some(AttributeKind::RustcObjectLifetimeDefault)
    }
}
