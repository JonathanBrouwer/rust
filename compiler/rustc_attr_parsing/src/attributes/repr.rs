use rustc_abi::{Align, Size};
use rustc_ast::{IntTy, LitIntType, LitKind, UintTy};
use rustc_hir::attrs::{IntType, ReprAttr};

use super::prelude::*;
use crate::session_diagnostics::{self, IncorrectReprFormatGenericCause, InvalidReprAlignForTarget};

/// Parse #[repr(...)] forms.
///
/// Valid repr contents: any of the primitive integral type names (see
/// `int_type_of_word`, below) to specify enum discriminant type; `C`, to use
/// the same discriminant size that the corresponding C enum would or C
/// structure layout, `packed` to remove padding, and `transparent` to delegate representation
/// concerns to the only non-ZST field.
// FIXME(jdonszelmann): is a vec the right representation here even? isn't it just a struct?
pub(crate) struct ReprParser;

impl<S: Stage> CombineAttributeParser<S> for ReprParser {
    type Item = (ReprAttr, Span);
    const PATH: &[Symbol] = &[sym::repr];
    const CONVERT: ConvertFn<Self::Item> =
        |items, first_span| AttributeKind::Repr { reprs: items, first_span };
    // FIXME(jdonszelmann): never used
    const TEMPLATE: AttributeTemplate = template!(
        List: &["C", "Rust", "transparent", "align(...)", "packed(...)", "<integer type>"],
        "https://doc.rust-lang.org/reference/type-layout.html#representations"
    );

    fn extend<'c>(
        cx: &'c mut AcceptContext<'_, '_, S>,
        args: &'c ArgParser<'_>,
    ) -> impl IntoIterator<Item = Self::Item> + 'c {
        let mut reprs = Vec::new();

        let Some(list) = args.list() else {
            cx.expected_list(cx.attr_span);
            return reprs;
        };

        if list.is_empty() {
            cx.warn_empty_attribute(cx.attr_span);
            return reprs;
        }

        for param in list.mixed() {
            if let Some(_) = param.lit() {
                cx.emit_err(session_diagnostics::ReprIdent { span: cx.attr_span });
                continue;
            }

            reprs.extend(
                param.meta_item().and_then(|mi| parse_repr(cx, &mi)).map(|r| (r, param.span())),
            );
        }

        reprs
    }

    //FIXME Still checked fully in `check_attr.rs`
    //This one is slightly more complicated because the allowed targets depend on the arguments
    const ALLOWED_TARGETS: AllowedTargets = AllowedTargets::AllowList(ALL_TARGETS);
}

macro_rules! int_pat {
    () => {
        sym::i8
            | sym::u8
            | sym::i16
            | sym::u16
            | sym::i32
            | sym::u32
            | sym::i64
            | sym::u64
            | sym::i128
            | sym::u128
            | sym::isize
            | sym::usize
    };
}

fn int_type_of_word(s: Symbol) -> Option<IntType> {
    use IntType::*;

    match s {
        sym::i8 => Some(SignedInt(IntTy::I8)),
        sym::u8 => Some(UnsignedInt(UintTy::U8)),
        sym::i16 => Some(SignedInt(IntTy::I16)),
        sym::u16 => Some(UnsignedInt(UintTy::U16)),
        sym::i32 => Some(SignedInt(IntTy::I32)),
        sym::u32 => Some(UnsignedInt(UintTy::U32)),
        sym::i64 => Some(SignedInt(IntTy::I64)),
        sym::u64 => Some(UnsignedInt(UintTy::U64)),
        sym::i128 => Some(SignedInt(IntTy::I128)),
        sym::u128 => Some(UnsignedInt(UintTy::U128)),
        sym::isize => Some(SignedInt(IntTy::Isize)),
        sym::usize => Some(UnsignedInt(UintTy::Usize)),
        _ => None,
    }
}

fn parse_repr<S: Stage>(
    cx: &AcceptContext<'_, '_, S>,
    param: &MetaItemParser<'_>,
) -> Option<ReprAttr> {
    use ReprAttr::*;
    // FIXME(jdonszelmann): invert the parsing here to match on the word first and then the
    // structure.
    let (name, ident_span) = if let Some(ident) = param.path().word() {
        (Some(ident.name), ident.span)
    } else {
        (None, DUMMY_SP)
    };

    let args = param.args();

    match (name, args) {
        (Some(sym::align), ArgParser::NoArgs) => {
            cx.emit_err(session_diagnostics::InvalidReprAlignNeedArg { span: ident_span });
            None
        }
        (Some(sym::align), ArgParser::List(l)) => {
            parse_repr_align(cx, l, param.span(), AlignKind::Align)
        }

        (Some(sym::packed), ArgParser::NoArgs) => Some(ReprPacked(Align::ONE)),
        (Some(sym::packed), ArgParser::List(l)) => {
            parse_repr_align(cx, l, param.span(), AlignKind::Packed)
        }

        (Some(name @ sym::align | name @ sym::packed), ArgParser::NameValue(l)) => {
            cx.emit_err(session_diagnostics::IncorrectReprFormatGeneric {
                span: param.span(),
                // FIXME(jdonszelmann) can just be a string in the diag type
                repr_arg: name,
                cause: IncorrectReprFormatGenericCause::from_lit_kind(
                    param.span(),
                    &l.value_as_lit().kind,
                    name,
                ),
            });
            None
        }

        (Some(sym::Rust), ArgParser::NoArgs) => Some(ReprRust),
        (Some(sym::C), ArgParser::NoArgs) => Some(ReprC),
        (Some(sym::simd), ArgParser::NoArgs) => Some(ReprSimd),
        (Some(sym::transparent), ArgParser::NoArgs) => Some(ReprTransparent),
        (Some(name @ int_pat!()), ArgParser::NoArgs) => {
            // int_pat!() should make sure it always parses
            Some(ReprInt(int_type_of_word(name).unwrap()))
        }

        (
            Some(
                name @ sym::Rust
                | name @ sym::C
                | name @ sym::simd
                | name @ sym::transparent
                | name @ int_pat!(),
            ),
            ArgParser::NameValue(_),
        ) => {
            cx.emit_err(session_diagnostics::InvalidReprHintNoValue { span: param.span(), name });
            None
        }
        (
            Some(
                name @ sym::Rust
                | name @ sym::C
                | name @ sym::simd
                | name @ sym::transparent
                | name @ int_pat!(),
            ),
            ArgParser::List(_),
        ) => {
            cx.emit_err(session_diagnostics::InvalidReprHintNoParen { span: param.span(), name });
            None
        }

        _ => {
            cx.emit_err(session_diagnostics::UnrecognizedReprHint { span: param.span() });
            None
        }
    }
}

enum AlignKind {
    Packed,
    Align,
}

fn parse_repr_align<S: Stage>(
    cx: &AcceptContext<'_, '_, S>,
    list: &MetaItemListParser<'_>,
    param_span: Span,
    align_kind: AlignKind,
) -> Option<ReprAttr> {
    use AlignKind::*;

    let Some(align) = list.single() else {
        match align_kind {
            Packed => {
                cx.emit_err(session_diagnostics::IncorrectReprFormatPackedOneOrZeroArg {
                    span: param_span,
                });
            }
            Align => {
                cx.emit_err(session_diagnostics::IncorrectReprFormatAlignOneArg {
                    span: param_span,
                });
            }
        }

        return None;
    };

    let Some(lit) = align.lit() else {
        match align_kind {
            Packed => {
                cx.emit_err(session_diagnostics::IncorrectReprFormatPackedExpectInteger {
                    span: align.span(),
                });
            }
            Align => {
                cx.emit_err(session_diagnostics::IncorrectReprFormatExpectInteger {
                    span: align.span(),
                });
            }
        }

        return None;
    };

    match parse_alignment(cx, &lit.kind) {
        Ok(literal) => Some(match align_kind {
            AlignKind::Packed => ReprAttr::ReprPacked(literal),
            AlignKind::Align => ReprAttr::ReprAlign(literal),
        }),
        Err(message) => {
            cx.emit_err(session_diagnostics::InvalidReprGeneric {
                span: lit.span,
                repr_arg: match align_kind {
                    Packed => "packed".to_string(),
                    Align => "align".to_string(),
                },
                error_part: message,
            });
            None
        }
    }
}

fn parse_alignment<S: Stage>(cx: &AcceptContext<'_, '_, S>, node: &LitKind) -> Result<Align, &'static str> {
    let LitKind::Int(literal, LitIntType::Unsuffixed) = node else {
        return Err("not an unsuffixed integer");
    };

    // `Align::from_bytes` accepts 0 as an input, check is_power_of_two() first
    if !literal.get().is_power_of_two() {
        return Err("not a power of two")
    }

    // Only possible error is larger than 2^29
    let align = literal
        .get()
        .try_into()
        .ok()
        .and_then(|v| Align::from_bytes(v).ok())
        .ok_or("larger than 2^29")?;

    // only do this check when <= 2^29 to prevent duplicate errors:
    // alignment greater than 2^29 not supported
    // alignment is too large for the current target
    let max = Size::from_bits(cx.sess.target.pointer_width).signed_int_max() as u64;
    if align.bytes() > max {
        cx.dcx().emit_err(InvalidReprAlignForTarget { span: cx.attr_span, size: max });
    }

    Ok(align)
}

/// Parse #[align(N)].
#[derive(Default)]
pub(crate) struct AlignParser(Option<(Align, Span)>);

impl AlignParser {
    const PATH: &'static [Symbol] = &[sym::rustc_align];
    const TEMPLATE: AttributeTemplate = template!(List: &["<alignment in bytes>"]);

    fn parse<'c, S: Stage>(
        &mut self,
        cx: &'c mut AcceptContext<'_, '_, S>,
        args: &'c ArgParser<'_>,
    ) {
        match args {
            ArgParser::NoArgs | ArgParser::NameValue(_) => {
                cx.expected_list(cx.attr_span);
            }
            ArgParser::List(list) => {
                let Some(align) = list.single() else {
                    cx.expected_single_argument(list.span);
                    return;
                };

                let Some(lit) = align.lit() else {
                    cx.emit_err(session_diagnostics::IncorrectReprFormatExpectInteger {
                        span: align.span(),
                    });

                    return;
                };

                match parse_alignment(cx, &lit.kind) {
                    Ok(literal) => self.0 = Ord::max(self.0, Some((literal, cx.attr_span))),
                    Err(message) => {
                        cx.emit_err(session_diagnostics::InvalidAlignmentValue {
                            span: lit.span,
                            error_part: message,
                        });
                    }
                }
            }
        }
    }
}

impl<S: Stage> AttributeParser<S> for AlignParser {
    const ATTRIBUTES: AcceptMapping<Self, S> = &[(Self::PATH, Self::TEMPLATE, Self::parse)];
    const ALLOWED_TARGETS: AllowedTargets = AllowedTargets::AllowList(&[
        Allow(Target::Fn),
        Allow(Target::Method(MethodKind::Inherent)),
        Allow(Target::Method(MethodKind::Trait { body: true })),
        Allow(Target::Method(MethodKind::TraitImpl)),
        Allow(Target::Method(MethodKind::Trait { body: false })),
        Allow(Target::ForeignFn),
    ]);

    fn finalize(self, _cx: &FinalizeContext<'_, '_, S>) -> Option<AttributeKind> {
        let (align, span) = self.0?;
        Some(AttributeKind::Align { align, span })
    }
}
