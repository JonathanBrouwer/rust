use rustc_attr_data_structures::AttributeKind;
use rustc_feature::{template, AttributeTemplate};
use rustc_span::{sym, Symbol};
use crate::attributes::{AttributeOrder, CombineAttributeParser, ConvertFn, OnDuplicate, SingleAttributeParser};
use crate::context::{AcceptContext, Stage};
use crate::parser::ArgParser;

pub(crate) struct MacroUseParser;

impl<S: Stage> CombineAttributeParser<S> for MacroUseParser {
    const PATH: &[Symbol] = &[sym::macro_use];
    type Item = ();
    const CONVERT: ConvertFn<Self::Item> = ();
    const TEMPLATE: AttributeTemplate = Default::default();

    fn extend<'c>(cx: &'c mut AcceptContext<'_, '_, S>, args: &'c ArgParser<'_>) -> impl IntoIterator<Item=Self::Item> + 'c {
        match args {
            
        }
        todo!()
    }
}