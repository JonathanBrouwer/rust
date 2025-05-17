use tracing::debug;
use rustc_hir::def::*;
use rustc_hir::def_id::LocalDefId;
use rustc_hir::{Block, Body, Expr, ExprKind, HirId, HirIdMap, HirIdSet, Item, ItemKind, Node, Stmt, StmtKind};
use rustc_middle::query::Providers;
use rustc_middle::ty::{self, RootVariableMinCaptureList, Ty, TyCtxt, TypeckResults, TypingEnv};
use rustc_session::lint;
use rustc_span::Span;
use crate::errors;

struct ReachabilityChecker<'tcx> {
    tcx: TyCtxt<'tcx>,
    typeck_results: &'tcx TypeckResults<'tcx>,
    typing_env: TypingEnv<'tcx>
}

impl ReachabilityChecker<'_> {

    fn warn_expr(&self, expr: &Expr, origin: &Expr) {
        let return_type = self.typeck_results.expr_ty(origin);
        self.tcx.emit_node_span_lint(
            lint::builtin::UNREACHABLE_CODE,
            expr.hir_id,
            expr.span,
            errors::UnreachableDueToUninhabited {
                expr: expr.span,
                orig: origin.span,
                descr: "expression",
                ty: return_type, //TODO wrong
            },
        );
    }

    fn warn_stmt(&self, stmt: &Stmt, origin: &Expr) {
        let return_type = self.typeck_results.expr_ty(origin);
        self.tcx.emit_node_span_lint(
            lint::builtin::UNREACHABLE_CODE,
            stmt.hir_id,
            stmt.span,
            errors::UnreachableDueToUninhabited {
                expr: stmt.span,
                orig: origin.span,
                descr: "statement",
                ty: return_type,
            },
        );
    }

    /// Returns whether `expr` diverges
    fn expr_diverges<'tcx>(&'tcx self, expr: &'tcx Expr<'tcx>) -> Option<&'tcx Expr<'tcx>> {
        match expr.kind {
            ExprKind::Block(b, _lbl) => {
                self.block_diverges(b)
            }
            ExprKind::MethodCall(_,f, args,_) | ExprKind::Call(f, args) => {
                let return_type = self.typeck_results.expr_ty(expr);
                let m = self.tcx.parent_module(expr.hir_id).to_def_id();
                (!return_type.is_inhabited_from(self.tcx, m, self.typing_env)).then_some(expr)
            },
            ExprKind::Lit(_) => {
                None
            },
            ExprKind::If(cond, if_expr, else_expr) => {
                if let Some(origin) = self.expr_diverges(cond) {
                    self.warn_expr(if_expr, origin);
                    if let Some(else_expr) = else_expr {
                        self.warn_expr(else_expr, origin);
                    }
                    return None
                }

                let if_diverges = self.expr_diverges(if_expr);
                let else_diverges = else_expr.map(|expr| self.expr_diverges(expr).is_some()).unwrap_or(false);
                if else_diverges {
                    if_diverges
                } else {
                    None
                }
            },
            _ => {
                None
                //TODO
            },
        }
    }

    fn block_diverges<'tcx>(&'tcx self, b: &'tcx Block) -> Option<&'tcx Expr> {
        let mut previous_diverged = None;

        for stmt in b.stmts {
            if let StmtKind::Item(..) = stmt.kind {
                continue
            }
            if let Some(origin) = previous_diverged {
                self.warn_stmt(stmt, origin);
                return None
            }
            previous_diverged = self.stmt_diverges(stmt);
        }

        if let Some(expr) = b.expr {
            if let Some(origin) = previous_diverged {
                self.warn_expr(expr, origin);
                return None
            }
            previous_diverged = self.expr_diverges(expr);
        }

        previous_diverged

    }

    fn stmt_diverges<'tcx>(&'tcx self, stmt: &'tcx Stmt) -> Option<&'tcx Expr> {
        match stmt.kind {
            StmtKind::Let(stmt) => {
                let init_diverges = stmt.init.and_then(|expr| self.expr_diverges(expr));
                let else_diverges = stmt.els.is_none_or(|block| self.block_diverges(block).is_some());
                if else_diverges {
                    init_diverges
                } else {
                    None
                }
            }
            StmtKind::Item(_item) => {
                None
            },
            StmtKind::Expr(expr) | StmtKind::Semi(expr) => self.expr_diverges(expr),
        }
    }
    fn check_item(&self, item: &Item) {
        match item.kind {
            ItemKind::Fn {body, ..} => {
                self.expr_diverges(self.tcx.hir_body(body).value);
            }

            _ => {
                //TODO
            }
        }
    }

    fn check_node(&self, node: &Node) {
        match node {
            Node::Item(item) => self.check_item(item),
            _ => {
                //TODO
            }
        }
    }
}



fn check_unreachable(tcx: TyCtxt<'_>, def_id: LocalDefId) {
    let typeck_results: &TypeckResults = tcx.typeck(def_id);
    let typing_env = ty::TypingEnv::non_body_analysis(tcx, def_id);


    let checker = ReachabilityChecker {
        tcx,
        typeck_results,
        typing_env
    };
    let node = checker.tcx.hir_node_by_def_id(def_id);
    checker.check_node(&node)
}

pub(crate) fn provide(providers: &mut Providers) {
    *providers = Providers { check_unreachable, ..*providers };
}
