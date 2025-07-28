//! This is in essence an (improved) duplicate of `rustc_ast/attr/mod.rs`.
//! That module is intended to be deleted in its entirety.
//!
//! FIXME(jdonszelmann): delete `rustc_ast/attr/mod.rs`

use std::fmt::{Debug, Display};
use std::iter::Peekable;

use rustc_ast::token::{self, Delimiter, Token};
use rustc_ast::tokenstream::{TokenStreamIter, TokenTree};
use rustc_ast::{AttrArgs, DelimArgs, Expr, ExprKind, LitKind, MetaItemLit, NormalAttr, Path};
use rustc_ast_pretty::pprust;
use rustc_hir::{self as hir, AttrPath};
use rustc_session::errors::report_lit_error;
use rustc_session::parse::ParseSess;
use rustc_span::{ErrorGuaranteed, Ident, Span, Symbol, kw, sym};
use crate::ShouldEmit;

pub struct SegmentIterator<'a> {
    offset: usize,
    path: &'a PathParser<'a>,
}

impl<'a> Iterator for SegmentIterator<'a> {
    type Item = &'a Ident;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.path.len() {
            return None;
        }

        let res = match self.path {
            PathParser::Ast(ast_path) => &ast_path.segments[self.offset].ident,
            PathParser::Attr(attr_path) => &attr_path.segments[self.offset],
        };

        self.offset += 1;
        Some(res)
    }
}

#[derive(Clone, Debug)]
pub enum PathParser<'a> {
    Ast(&'a Path),
    Attr(AttrPath),
}

impl<'a> PathParser<'a> {
    pub fn get_attribute_path(&self) -> hir::AttrPath {
        AttrPath {
            segments: self.segments().copied().collect::<Vec<_>>().into_boxed_slice(),
            span: self.span(),
        }
    }

    pub fn segments(&'a self) -> impl Iterator<Item = &'a Ident> {
        SegmentIterator { offset: 0, path: self }
    }

    pub fn span(&self) -> Span {
        match self {
            PathParser::Ast(path) => path.span,
            PathParser::Attr(attr_path) => attr_path.span,
        }
    }

    pub fn len(&self) -> usize {
        match self {
            PathParser::Ast(path) => path.segments.len(),
            PathParser::Attr(attr_path) => attr_path.segments.len(),
        }
    }

    pub fn segments_is(&self, segments: &[Symbol]) -> bool {
        self.len() == segments.len() && self.segments().zip(segments).all(|(a, b)| a.name == *b)
    }

    pub fn word(&self) -> Option<Ident> {
        (self.len() == 1).then(|| **self.segments().next().as_ref().unwrap())
    }

    pub fn word_sym(&self) -> Option<Symbol> {
        self.word().map(|ident| ident.name)
    }

    /// Asserts that this MetaItem is some specific word.
    ///
    /// See [`word`](Self::word) for examples of what a word is.
    pub fn word_is(&self, sym: Symbol) -> bool {
        self.word().map(|i| i.name == sym).unwrap_or(false)
    }

    /// Checks whether the first segments match the givens.
    ///
    /// Unlike [`segments_is`](Self::segments_is),
    /// `self` may contain more segments than the number matched  against.
    pub fn starts_with(&self, segments: &[Symbol]) -> bool {
        segments.len() < self.len() && self.segments().zip(segments).all(|(a, b)| a.name == *b)
    }
}

impl Display for PathParser<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathParser::Ast(path) => write!(f, "{}", pprust::path_to_string(path)),
            PathParser::Attr(attr_path) => write!(f, "{attr_path}"),
        }
    }
}

#[derive(Clone, Debug)]
#[must_use]
pub enum ArgParser<'a> {
    NoArgs,
    List(MetaItemListParser<'a>),
    NameValue(NameValueParser),
}

impl<'a> ArgParser<'a> {
    pub fn span(&self) -> Option<Span> {
        match self {
            Self::NoArgs => None,
            Self::List(l) => Some(l.span),
            Self::NameValue(n) => Some(n.value_span.with_lo(n.eq_span.lo())),
        }
    }

    pub fn from_attr_args(value: &'a AttrArgs, psess: &ParseSess, should_emit: ShouldEmit) -> Self {
        match value {
            AttrArgs::Empty => Self::NoArgs,
            AttrArgs::Delimited(args) if args.delim == Delimiter::Parenthesis => {
                Self::List(MetaItemListParser::new(args, psess, should_emit ))
            }
            AttrArgs::Delimited(args) => {
                Self::List(MetaItemListParser { sub_parsers: vec![], span: args.dspan.entire() })
            }
            AttrArgs::Eq { eq_span, expr } => Self::NameValue(NameValueParser {
                eq_span: *eq_span,
                value: expr_to_lit(psess, &expr, should_emit),
                value_span: expr.span,
            }),
        }
    }

    /// Asserts that this MetaItem is a list
    ///
    /// Some examples:
    ///
    /// - `#[allow(clippy::complexity)]`: `(clippy::complexity)` is a list
    /// - `#[rustfmt::skip::macros(target_macro_name)]`: `(target_macro_name)` is a list
    pub fn list(&self) -> Option<&MetaItemListParser<'a>> {
        match self {
            Self::List(l) => Some(l),
            Self::NameValue(_) | Self::NoArgs => None,
        }
    }

    /// Asserts that this MetaItem is a name-value pair.
    ///
    /// Some examples:
    ///
    /// - `#[clippy::cyclomatic_complexity = "100"]`: `clippy::cyclomatic_complexity = "100"` is a name value pair,
    ///   where the name is a path (`clippy::cyclomatic_complexity`). You already checked the path
    ///   to get an `ArgParser`, so this method will effectively only assert that the `= "100"` is
    ///   there
    /// - `#[doc = "hello"]`: `doc = "hello`  is also a name value pair
    pub fn name_value(&self) -> Option<&NameValueParser> {
        match self {
            Self::NameValue(n) => Some(n),
            Self::List(_) | Self::NoArgs => None,
        }
    }

    /// Assert that there were no args.
    /// If there were, get a span to the arguments
    /// (to pass to [`AcceptContext::expected_no_args`](crate::context::AcceptContext::expected_no_args)).
    pub fn no_args(&self) -> Result<(), Span> {
        match self {
            Self::NoArgs => Ok(()),
            Self::List(args) => Err(args.span),
            Self::NameValue(args) => Err(args.eq_span.to(args.value_span)),
        }
    }
}

/// Inside lists, values could be either literals, or more deeply nested meta items.
/// This enum represents that.
///
/// Choose which one you want using the provided methods.
#[derive(Debug, Clone)]
pub enum MetaItemOrLitParser<'a> {
    MetaItemParser(MetaItemParser<'a>),
    Lit(MetaItemLit),
    Err(Span, ErrorGuaranteed),
}

impl<'a> MetaItemOrLitParser<'a> {
    pub fn span(&self) -> Span {
        match self {
            MetaItemOrLitParser::MetaItemParser(generic_meta_item_parser) => {
                generic_meta_item_parser.span()
            }
            MetaItemOrLitParser::Lit(meta_item_lit) => meta_item_lit.span,
            MetaItemOrLitParser::Err(span, _) => *span,
        }
    }

    pub fn lit(&self) -> Option<&MetaItemLit> {
        match self {
            MetaItemOrLitParser::Lit(meta_item_lit) => Some(meta_item_lit),
            _ => None,
        }
    }

    pub fn meta_item(&self) -> Option<&MetaItemParser<'a>> {
        match self {
            MetaItemOrLitParser::MetaItemParser(parser) => Some(parser),
            _ => None,
        }
    }
}

/// Utility that deconstructs a MetaItem into usable parts.
///
/// MetaItems are syntactically extremely flexible, but specific attributes want to parse
/// them in custom, more restricted ways. This can be done using this struct.
///
/// MetaItems consist of some path, and some args. The args could be empty. In other words:
///
/// - `name` -> args are empty
/// - `name(...)` -> args are a [`list`](ArgParser::list), which is the bit between the parentheses
/// - `name = value`-> arg is [`name_value`](ArgParser::name_value), where the argument is the
///   `= value` part
///
/// The syntax of MetaItems can be found at <https://doc.rust-lang.org/reference/attributes.html>
#[derive(Clone)]
pub struct MetaItemParser<'a> {
    path: PathParser<'a>,
    args: ArgParser<'a>,
}

impl<'a> Debug for MetaItemParser<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetaItemParser")
            .field("path", &self.path)
            .field("args", &self.args)
            .finish()
    }
}

impl<'a> MetaItemParser<'a> {
    /// Create a new parser from a [`NormalAttr`], which is stored inside of any
    /// [`ast::Attribute`](rustc_ast::Attribute)
    pub fn from_attr(attr: &'a NormalAttr, psess: &ParseSess, should_emit: ShouldEmit) -> Self {
        Self {
            path: PathParser::Ast(&attr.item.path),
            args: ArgParser::from_attr_args(&attr.item.args, psess, should_emit),
        }
    }
}

impl<'a> MetaItemParser<'a> {
    pub fn span(&self) -> Span {
        if let Some(other) = self.args.span() {
            self.path.span().with_hi(other.hi())
        } else {
            self.path.span()
        }
    }

    /// Gets just the path, without the args. Some examples:
    ///
    /// - `#[rustfmt::skip]`: `rustfmt::skip` is a path
    /// - `#[allow(clippy::complexity)]`: `clippy::complexity` is a path
    /// - `#[inline]`: `inline` is a single segment path
    pub fn path(&self) -> &PathParser<'a> {
        &self.path
    }

    /// Gets just the args parser, without caring about the path.
    pub fn args(&self) -> &ArgParser<'a> {
        &self.args
    }

    /// Asserts that this MetaItem starts with a word, or single segment path.
    ///
    /// Some examples:
    /// - `#[inline]`: `inline` is a word
    /// - `#[rustfmt::skip]`: `rustfmt::skip` is a path,
    ///   and not a word and should instead be parsed using [`path`](Self::path)
    pub fn word_is(&self, sym: Symbol) -> Option<&ArgParser<'a>> {
        self.path().word_is(sym).then(|| self.args())
    }
}

#[derive(Clone)]
pub struct NameValueParser {
    pub eq_span: Span,
    value: MetaItemLit,
    pub value_span: Span,
}

impl Debug for NameValueParser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NameValueParser")
            .field("eq_span", &self.eq_span)
            .field("value", &self.value)
            .field("value_span", &self.value_span)
            .finish()
    }
}

impl NameValueParser {
    pub fn value_as_lit(&self) -> &MetaItemLit {
        &self.value
    }

    pub fn value_as_str(&self) -> Option<Symbol> {
        self.value_as_lit().kind.str()
    }
}

fn expr_to_lit(psess: &ParseSess, expr: &Expr, should_emit: ShouldEmit) -> MetaItemLit {
    match expr.kind {
        ExprKind::Lit(token_lit) => lit_token_to_lit(psess, token_lit, expr.span, should_emit),
        _ => {
            let msg = "attribute value must be a literal";
            let mut err = psess.dcx().struct_span_err(expr.span, msg);
            if let ExprKind::Err(_) = expr.kind {
                err.downgrade_to_delayed_bug();
            }
            let guar = err.emit_unless_delay(!should_emit.should_emit());
            MetaItemLit { symbol: sym::dummy, suffix: None, kind: LitKind::Err(guar), span: expr.span }
        }
    }
}

fn lit_token_to_lit(psess: &ParseSess, token_lit: token::Lit, span: Span, should_emit: ShouldEmit) -> MetaItemLit {
    match MetaItemLit::from_token_lit(token_lit, span) {
        Ok(lit) => {
            if token_lit.suffix.is_some() {
                let mut err = psess.dcx().struct_span_err(
                    span,
                    "suffixed literals are not allowed in attributes",
                );
                err.help(
                    "instead of using a suffixed literal (`1u8`, `1.0f32`, etc.), \
                                    use an unsuffixed version (`1`, `1.0`, etc.)",
                );
                err.emit_unless_delay(!should_emit.should_emit());
            }
            lit
        },
        Err(err) => {
            let guar = if should_emit.should_emit() {
                report_lit_error(psess, err, token_lit, span)
            } else {
                psess.dcx().span_delayed_bug(
                    span,
                    "expr in place where literal is expected (builtin attr parsing)",
                )
            };
            MetaItemLit { symbol: token_lit.symbol, suffix: token_lit.suffix, kind: LitKind::Err(guar), span }
        }
    }
}

struct MetaItemListParserContext<'a, 'sess> {
    // the tokens inside the delimiters, so `#[some::attr(a b c)]` would have `a b c` inside
    inside_delimiters: Peekable<TokenStreamIter<'a>>,
    psess: &'sess ParseSess,
    should_emit: ShouldEmit
}

impl<'a, 'sess> MetaItemListParserContext<'a, 'sess> {
    fn done(&mut self) -> bool {
        self.inside_delimiters.peek().is_none()
    }

    fn next_path(&mut self) -> Result<AttrPath, ErrorGuaranteed> {
        // FIXME: Share code with `parse_path`.
        let tt = self.inside_delimiters.next().map(|tt| TokenTree::uninterpolate(tt));

        match tt.as_deref()? {
            &TokenTree::Token(
                Token { kind: ref kind @ (token::Ident(..) | token::PathSep), span },
                _,
            ) => {
                // here we have either an ident or pathsep `::`.

                let mut segments = if let &token::Ident(name, _) = kind {
                    // when we lookahead another pathsep, more path's coming
                    if let Some(TokenTree::Token(Token { kind: token::PathSep, .. }, _)) =
                        self.inside_delimiters.peek()
                    {
                        self.inside_delimiters.next();
                        vec![Ident::new(name, span)]
                    } else {
                        // else we have a single identifier path, that's all
                        return Ok(AttrPath {
                            segments: vec![Ident::new(name, span)].into_boxed_slice(),
                            span,
                        });
                    }
                } else {
                    // if `::` is all we get, we just got a path root
                    vec![Ident::new(kw::PathRoot, span)]
                };

                // one segment accepted. accept n more
                loop {
                    // another ident?
                    if let Some(&TokenTree::Token(Token { kind: token::Ident(name, _), span }, _)) =
                        self.inside_delimiters
                            .next()
                            .map(|tt| TokenTree::uninterpolate(tt))
                            .as_deref()
                    {
                        segments.push(Ident::new(name, span));
                    } else {
                        return None;
                    }
                    // stop unless we see another `::`
                    if let Some(TokenTree::Token(Token { kind: token::PathSep, .. }, _)) =
                        self.inside_delimiters.peek()
                    {
                        self.inside_delimiters.next();
                    } else {
                        break;
                    }
                }
                let span = span.with_hi(segments.last().unwrap().span.hi());
                Ok(AttrPath { segments: segments.into_boxed_slice(), span })
            }
            TokenTree::Token(Token { kind, .. }, _) if kind.is_delim() => None,
            _ => {
                // malformed attributes can get here. We can't crash, but somewhere else should've
                // already warned for this.
                None
            }
        }
    }

    fn value(&mut self) -> Option<MetaItemLit> {
        match self.inside_delimiters.next() {
            Some(TokenTree::Delimited(.., Delimiter::Invisible(_), inner_tokens)) => {
                MetaItemListParserContext {
                    inside_delimiters: inner_tokens.iter().peekable(),
                    psess: self.psess,
                    should_emit: self.should_emit
                }
                .value()
            }
            Some(TokenTree::Token(token, _)) => MetaItemLit::from_token(token),
            _ => None,
        }
    }

    /// parses one element on the inside of a list attribute like `#[my_attr( <insides> )]`
    ///
    /// parses a path followed be either:
    /// 1. nothing (a word attr)
    /// 2. a parenthesized list
    /// 3. an equals sign and a literal (name-value)
    ///
    /// Can also parse *just* a literal. This is for cases like as `#[my_attr("literal")]`
    /// where no path is given before the literal
    ///
    /// Some exceptions too for interpolated attributes which are already pre-processed
    fn next(&mut self) -> Result<MetaItemOrLitParser<'a>, ErrorGuaranteed> {
        // a list element is either a literal
        if let Some(TokenTree::Token(token, _)) = self.inside_delimiters.peek()
            && let Some(token_lit) = token::Lit::from_token(token)
        {
            self.inside_delimiters.next();
            return Ok(MetaItemOrLitParser::Lit(lit_token_to_lit(self.psess, token_lit, token.span, self.should_emit)));
        } else if let Some(TokenTree::Delimited(.., Delimiter::Invisible(_), inner_tokens)) =
            self.inside_delimiters.peek()
        {
            self.inside_delimiters.next();
            return MetaItemListParserContext {
                inside_delimiters: inner_tokens.iter().peekable(),
                psess: self.psess,
                should_emit: self.should_emit
            }
            .next();
        }

        // or a path.
        let path = self.next_path()?;

        // Paths can be followed by:
        // - `(more meta items)` (another list)
        // - `= lit` (a name-value)
        // - nothing
        Some(MetaItemOrLitParser::MetaItemParser(match self.inside_delimiters.peek() {
            Some(TokenTree::Delimited(dspan, _, Delimiter::Parenthesis, inner_tokens)) => {
                self.inside_delimiters.next();

                MetaItemParser {
                    path: PathParser::Attr(path),
                    args: ArgParser::List(MetaItemListParser::new_tts(
                        inner_tokens.iter(),
                        dspan.entire(),
                        self.psess,
                        self.should_emit
                    )),
                }
            }
            Some(TokenTree::Delimited(_, ..)) => {
                self.inside_delimiters.next();
                // self.dcx.span_delayed_bug(span.entire(), "wrong delimiters");
                return Err(ErrorGuaranteed::unchecked_error_guaranteed());
            }
            Some(TokenTree::Token(Token { kind: token::Eq, span }, _)) => {
                self.inside_delimiters.next();
                let value = self.value()?;
                MetaItemParser {
                    path: PathParser::Attr(path),
                    args: ArgParser::NameValue(NameValueParser {
                        eq_span: *span,
                        value_span: value.span,
                        value,
                    }),
                }
            }
            _ => MetaItemParser { path: PathParser::Attr(path), args: ArgParser::NoArgs },
        }))
    }

    fn parse(mut self, span: Span) -> MetaItemListParser<'a> {
        let mut sub_parsers = Vec::new();

        while !self.done() {
            let Some(n) = self.next() else {
                continue;
            };
            sub_parsers.push(n);

            match self.inside_delimiters.peek() {
                None | Some(TokenTree::Token(Token { kind: token::Comma, .. }, _)) => {
                    self.inside_delimiters.next();
                }
                Some(_) => {}
            }
        }

        MetaItemListParser { sub_parsers, span }
    }
}

#[derive(Debug, Clone)]
pub struct MetaItemListParser<'a> {
    sub_parsers: Vec<MetaItemOrLitParser<'a>>,
    pub span: Span,
}

impl<'a> MetaItemListParser<'a> {
    fn new(delim: &'a DelimArgs, psess: &ParseSess, should_emit: ShouldEmit) -> Self {
        MetaItemListParser::new_tts(delim.tokens.iter(), delim.dspan.entire(), psess, should_emit)
    }

    fn new_tts(tts: TokenStreamIter<'a>, span: Span, psess: &ParseSess, should_emit: ShouldEmit) -> Self {
        MetaItemListParserContext { inside_delimiters: tts.peekable(), psess, should_emit }.parse(span)
    }

    /// Lets you pick and choose as what you want to parse each element in the list
    pub fn mixed(&self) -> impl Iterator<Item = &MetaItemOrLitParser<'a>> {
        self.sub_parsers.iter()
    }

    pub fn len(&self) -> usize {
        self.sub_parsers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns Some if the list contains only a single element.
    ///
    /// Inside the Some is the parser to parse this single element.
    pub fn single(&self) -> Option<&MetaItemOrLitParser<'a>> {
        let mut iter = self.mixed();
        iter.next().filter(|_| iter.next().is_none())
    }
}
