use tracing::debug;
use rustc_hir::def::*;
use rustc_hir::def_id::LocalDefId;
use rustc_hir::{Block, Body, Expr, ExprKind, HirId, HirIdMap, HirIdSet, Item, ItemKind, Node, Stmt, StmtKind};
use rustc_middle::query::Providers;
use rustc_middle::ty::{self, RootVariableMinCaptureList, Ty, TyCtxt, TypeckResults, TypingEnv};
use rustc_session::lint;
use crate::errors;

struct ReachabilityChecker<'tcx> {
    tcx: TyCtxt<'tcx>,
    typeck_results: &'tcx TypeckResults<'tcx>,
    typing_env: TypingEnv<'tcx>
}

impl ReachabilityChecker<'_> {

    fn warn_expr(&self, expr: &Expr) {
        let return_type = self.typeck_results.expr_ty(expr);
        self.tcx.emit_node_span_lint(
            lint::builtin::UNREACHABLE_CODE,
            expr.hir_id,
            expr.span,
            errors::UnreachableDueToUninhabited {
                expr: expr.span,
                orig: expr.span, // TODO wrong
                descr: "expression",
                ty: return_type, //TODO wrong
            },
        );
    }

    fn warn_stmt(&self, stmt: &Stmt) {
        self.tcx.emit_node_span_lint(
            lint::builtin::UNREACHABLE_CODE,
            stmt.hir_id,
            stmt.span,
            errors::UnreachableDueToUninhabited {
                expr: stmt.span,
                orig: stmt.span, // TODO wrong
                descr: "statement",
                ty: self.tcx.types.bool, //TODO wrong
            },
        );
    }

    /// Returns whether `expr` diverges
    fn expr_diverges(&self, expr: &Expr) -> bool {
        match expr.kind {
            ExprKind::Block(b, _lbl) => {
                self.block_diverges(b)
            }
            ExprKind::MethodCall(_,f, args,_) | ExprKind::Call(f, args) => {
                let return_type = self.typeck_results.expr_ty(expr);
                let m = self.tcx.parent_module(expr.hir_id).to_def_id();
                !return_type.is_inhabited_from(self.tcx, m, self.typing_env)
            },
            ExprKind::Lit(_) => {
                false
            },
            ExprKind::If(cond, if_expr, else_expr) => {
                if self.expr_diverges(cond) {
                    self.warn_expr(if_expr);
                    if let Some(else_expr) = else_expr {
                        self.warn_expr(else_expr);
                    }
                }
                self.expr_diverges(if_expr) && else_expr.map(|expr| self.expr_diverges(expr)).unwrap_or(false)
            },
            _ => {

                false
                //TODO
            },
        }
    }

    fn block_diverges(&self, b: &Block) -> bool {
        let mut previous_diverged = false;


        for stmt in b.stmts {
            if let StmtKind::Item(..) = stmt.kind {
                continue
            }
            if previous_diverged {
                self.warn_stmt(stmt);
                return true
            }
            previous_diverged = self.stmt_diverges(stmt);
        }

        if let Some(expr) = b.expr {
            if previous_diverged {
                self.warn_expr(expr);
                return true
            }
            previous_diverged = self.expr_diverges(expr);
        }

        previous_diverged

    }

    fn stmt_diverges(&self, stmt: &Stmt) -> bool {
        match stmt.kind {
            StmtKind::Let(stmt) => {
                let init_diverges = stmt.init.is_some_and(|expr| self.expr_diverges(expr));
                let else_diverges = stmt.els.is_none_or(|block| self.block_diverges(block));
                init_diverges && else_diverges
            }
            StmtKind::Item(item) => {
                false
            },
            StmtKind::Expr(expr) | StmtKind::Semi(expr) => self.expr_diverges(expr),
        }
    }
    fn check_item(&self, item: &Item<'_>) {
        match item.kind {
            ItemKind::Fn {body, ..} => {
                self.expr_diverges(self.tcx.hir_body(body).value);
            }

            _ => {
                //TODO
            }
        }
    }

    fn check_node(&self, node: &Node<'_>) {
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
    checker.check_node(&checker.tcx.hir_node_by_def_id(def_id))
}

pub(crate) fn provide(providers: &mut Providers) {
    *providers = Providers { check_unreachable, ..*providers };
}
