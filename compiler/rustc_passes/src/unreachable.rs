use tracing::debug;
use rustc_ast::{BinOpKind, InlineAsmOptions};
use rustc_hir::def::*;
use rustc_hir::def_id::LocalDefId;
use rustc_hir::{Block, Body, Expr, ExprKind, HirId, HirIdMap, HirIdSet, Item, ItemKind, Node, Stmt, StmtKind, StructTailExpr};
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

enum DivergingReason {

}

impl ReachabilityChecker<'_> {

    fn warn_expr(&self, expr: &Expr, origin: &Expr, descr: &'static str) {
        let return_type = self.typeck_results.expr_ty(origin);
        self.tcx.emit_node_span_lint(
            lint::builtin::UNREACHABLE_CODE,
            expr.hir_id,
            expr.span,
            errors::UnreachableDueToUninhabited {
                expr: expr.span,
                orig: origin.span,
                descr,
                ty: return_type,
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
                let mut diverges = None;
                for arg in args {
                    if let Some(div) = self.expr_diverges(arg) {
                        diverges = Some(div);
                    }
                }
                if let Some(diverges) = diverges {
                    self.warn_expr(f, diverges, "call")
                }

                self.check_expression_resolving_type_is_not_inhabited(expr)
            },
            ExprKind::If(cond, if_expr, else_expr) => {
                if let Some(origin) = self.expr_diverges(cond) {
                    self.warn_expr(if_expr, origin, "block in `if`");
                    if let Some(else_expr) = else_expr {
                        self.warn_expr(else_expr, origin, "block in `if`");
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
            ExprKind::Loop(block, _,_, _span) => {
                let _ = self.block_diverges(block);

                self.check_expression_resolving_type_is_not_inhabited(expr)
            }
            ExprKind::Continue(_) => {
                Some(expr)
            }
            ExprKind::Ret(sub) | ExprKind::Break(_, sub) => {
                if let Some(sub) = sub && let Some(origin) = self.expr_diverges(sub) {
                    self.warn_expr(expr, origin, "expression")
                }
                Some(expr)
            },
            ExprKind::Yield(sub,..) => {
                self.expr_diverges(sub);
                None
            }
            ExprKind::InlineAsm(_) => {
                self.check_expression_resolving_type_is_not_inhabited(expr)
            },
            ExprKind::Closure(closure) => {
                self.expr_diverges(self.tcx.hir_body(closure.body).value);
                None
            }
            ExprKind::Let(let_expr) => {
                self.expr_diverges(let_expr.init)
            }
            ExprKind::DropTemps(drop_expr) => {
                self.expr_diverges(drop_expr)
            }
            ExprKind::Match(match_expr, arms, _) => {
                if let Some(match_orig) = self.expr_diverges(match_expr) {
                    self.warn_expr(expr, match_orig, "expression");
                }

                let all_diverge = arms.iter().all(|arm| {
                    let arm_diverges = if let Some(origin) = arm.guard.and_then(|guard| self.expr_diverges(guard)) {
                        self.warn_expr(arm.body, origin, "expression");
                        true
                    } else {
                        false
                    };

                    self.expr_diverges(arm.body).is_some() || arm_diverges
                });

                //TODO this does not produce the nice error message (see expr_match.rs test)
                if all_diverge {
                    Some(expr)
                } else {
                    None
                }
            }
            ExprKind::Binary(bin_op, left,right) => {
                let left_origin = self.expr_diverges(left);
                let right_origin = self.expr_diverges(right);
                // For short-circuiting binary operators, the entire expression only diverges if the left operand does
                let combined = if bin_op.node == BinOpKind::And || bin_op.node == BinOpKind::Or {
                    left_origin
                } else {
                    left_origin.or(right_origin)
                };

                if let Some(origin) = combined {
                    self.warn_expr(expr, origin, "expression")
                }
                None
            },
            ExprKind::Unary(_, inner) | ExprKind::Type(inner, _) => {
                if let Some(origin) = self.expr_diverges(inner) {
                    self.warn_expr(expr, origin, "expression")
                }
                None
            }
            ExprKind::Array(elements) | ExprKind::Tup(elements) => {
                for element in elements {
                    if let Some(origin) = self.expr_diverges(element) {
                        self.warn_expr(expr, origin, "expression");
                        break
                    }
                }
                None
            }
            ExprKind::Cast(sub, _) | ExprKind::UnsafeBinderCast(_, sub,_) => {
                if let Some(origin) = self.expr_diverges(sub) {
                    self.warn_expr(expr, origin, "expression");
                }
                None
            }
            ExprKind::Assign(left, right, _) | ExprKind::AssignOp(_, left, right) => {
                let left_origin = self.expr_diverges(left);
                let right_origin = self.expr_diverges(right);
                if let Some(origin) = left_origin.or(right_origin) {
                    self.warn_expr(expr, origin, "expression")
                }
                None
            }
            ExprKind::Repeat(left, _right) => {
                // Constant expressions can not have the type `never`, so we don't have to worry about the right side
                if let Some(origin) = self.expr_diverges(left) {
                    self.warn_expr(expr, origin, "expression")
                }
                None
            }
            ExprKind::Struct(_, fields, tail) => {
                for field in fields {
                    if let Some(origin) = self.expr_diverges(field.expr) {
                        self.warn_expr(expr, origin, "expression");
                        return None
                    }
                }
                if let StructTailExpr::Base(tail) = tail && let Some(origin) = self.expr_diverges(tail) {
                    self.warn_expr(expr, origin, "expression")
                }
                None
            }
            ExprKind::Lit(_) | _ => {
                None
                //TODO
            },
        }
    }


    /// Returns `expr` if expr diverges, done by checking the type of expr
    fn check_expression_resolving_type_is_not_inhabited<'tcx>(&self, expr: &'tcx Expr<'tcx>) -> Option<&'tcx Expr<'tcx>> {
        let return_type = self.typeck_results.expr_ty(expr);
        let m = self.tcx.parent_module(expr.hir_id).to_def_id();
        (!return_type.is_inhabited_from(self.tcx, m, self.typing_env)).then_some(expr)
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
                self.warn_expr(expr, origin, "expression");
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
