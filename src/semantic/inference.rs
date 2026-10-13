use std::collections::{BTreeSet, HashMap, HashSet};

use super::{FunctionId, SemanticAnalyzer, SemanticError};
use crate::{
    ast::{Block, Expr, Function, Program, StatementKind, ValueBlock},
    tokens::Span,
};

impl SemanticAnalyzer {
    pub(super) fn infer_results(&mut self, program: &Program) -> Result<(), SemanticError> {
        if program
            .functions
            .iter()
            .all(|function| function.return_type.is_some())
        {
            return Ok(());
        }

        let names: HashMap<_, _> = program
            .functions
            .iter()
            .enumerate()
            .map(|(index, function)| (function.name.as_str(), index))
            .collect();
        let graph: Vec<_> = program
            .functions
            .iter()
            .map(|function| {
                let mut calls = Vec::new();
                let mut walker = LexicalWalker::new(|name, _span| {
                    if let Some(&callee) = names.get(name) {
                        calls.push(Call { callee });
                        Some(&program.functions[callee])
                    } else {
                        None
                    }
                });

                walker.function(function);

                calls
            })
            .collect();
        let recursive = recursive_members(&graph);

        // Recursion uses every resolved call, including annotated barriers and
        // skipped/unreachable syntax, independently of result scheduling.
        for (index, function) in program.functions.iter().enumerate() {
            if function.return_type.is_some() {
                continue;
            }
            if recursive[index] {
                return Err(SemanticError::RecursiveResultAnnotationRequired {
                    name: function.name.clone(),
                    span: function.span,
                });
            }
        }

        let mut pending = vec![0; graph.len()];
        let mut callers = vec![Vec::new(); graph.len()];
        let mut ready = BTreeSet::new();

        for (index, function) in program.functions.iter().enumerate() {
            if function.return_type.is_some() {
                continue;
            }

            // All calls in a probed body need concrete signatures, even when
            // their values are discarded. Duplicate edges are counted and
            // released together; annotated signatures are already available.
            for call in &graph[index] {
                if program.functions[call.callee].return_type.is_none() {
                    pending[index] += 1;
                    callers[call.callee].push(index);
                }
            }

            if pending[index] == 0 {
                ready.insert(index);
            }
        }

        while let Some(index) = ready.pop_first() {
            let function = &program.functions[index];
            let return_type = self.infer_result(function, FunctionId(index))?;

            self.functions
                .get_mut(&function.name)
                .expect("inference follows declaration registration")
                .return_type = return_type;

            for &caller in &callers[index] {
                pending[caller] -= 1;

                if pending[caller] == 0 {
                    ready.insert(caller);
                }
            }
        }

        debug_assert!(pending.iter().all(|&count| count == 0));

        Ok(())
    }
}

struct Call {
    callee: usize,
}

// Iterative Kosaraju traversal avoids consuming the Rust call stack for long
// declaration graphs. Traversal order never decides the diagnostic: callers
// select the first offending unannotated declaration in source order.
fn recursive_members(graph: &[Vec<Call>]) -> Vec<bool> {
    let mut seen = vec![false; graph.len()];
    let mut finished = Vec::with_capacity(graph.len());

    for root in 0..graph.len() {
        if seen[root] {
            continue;
        }
        seen[root] = true;

        let mut stack = vec![(root, 0)];

        while let Some((node, next)) = stack.last_mut() {
            if let Some(call) = graph[*node].get(*next) {
                *next += 1;

                if !seen[call.callee] {
                    seen[call.callee] = true;
                    stack.push((call.callee, 0));
                }
            } else {
                finished.push(*node);
                stack.pop();
            }
        }
    }

    let mut reverse = vec![Vec::new(); graph.len()];

    for (caller, calls) in graph.iter().enumerate() {
        for call in calls {
            reverse[call.callee].push(caller);
        }
    }

    let mut components = vec![usize::MAX; graph.len()];
    let mut sizes = Vec::new();

    for root in finished.into_iter().rev() {
        if components[root] != usize::MAX {
            continue;
        }
        let component = sizes.len();
        let mut size = 0;
        let mut stack = vec![root];

        components[root] = component;

        while let Some(node) = stack.pop() {
            size += 1;

            for &caller in &reverse[node] {
                if components[caller] == usize::MAX {
                    components[caller] = component;
                    stack.push(caller);
                }
            }
        }
        sizes.push(size);
    }

    graph
        .iter()
        .enumerate()
        .map(|(index, calls)| {
            sizes[components[index]] > 1 || calls.iter().any(|call| call.callee == index)
        })
        .collect()
}

pub(super) fn has_value_return(function: &Function) -> bool {
    let mut walker = LexicalWalker::new(|_, _| None);

    walker.function(function);

    walker.has_value_return
}

// This is lexical discovery, not a second expression checker or a return-type
// constraint bag. Visit skipped/unreachable children and mirror checker scopes.
struct LexicalWalker<'a, F> {
    scopes: Vec<HashSet<&'a str>>,
    call: F,
    has_value_return: bool,
}

impl<'a, F: FnMut(&'a str, Span) -> Option<&'a Function>> LexicalWalker<'a, F> {
    fn new(call: F) -> Self {
        Self {
            scopes: Vec::new(),
            call,
            has_value_return: false,
        }
    }

    fn function(&mut self, function: &'a Function) {
        self.scopes.push(
            function
                .params
                .iter()
                .map(|param| param.name.as_str())
                .collect(),
        );
        self.block(&function.body);
        self.scopes.pop();
    }

    fn block(&mut self, block: &'a Block) {
        for statement in block {
            let span = statement.span;

            match &statement.kind {
                StatementKind::Let { name, value, .. } => {
                    self.expr(value, span);
                    self.scopes
                        .last_mut()
                        .expect("block has a lexical scope")
                        .insert(name);
                }
                StatementKind::Assignment { target, value }
                | StatementKind::CompoundAssignment { target, value, .. } => {
                    self.expr(target, span);
                    self.expr(value, span);
                }
                StatementKind::If { cond, then, else_ } => {
                    self.expr(cond, span);
                    self.scoped_block(then);

                    if let Some(block) = else_ {
                        self.scoped_block(block);
                    }
                }
                StatementKind::While { cond, body } => {
                    self.expr(cond, span);
                    self.scoped_block(body);
                }
                StatementKind::For {
                    name,
                    iterable,
                    body,
                } => {
                    self.expr(iterable, span);
                    self.scopes.push(HashSet::from([name.as_str()]));
                    self.scoped_block(body);
                    self.scopes.pop();
                }
                StatementKind::Loop { body } => self.scoped_block(body),
                StatementKind::Return(Some(value)) => {
                    self.has_value_return = true;
                    self.expr(value, span);
                }
                StatementKind::Expression(value) | StatementKind::BreakValue(value) => {
                    self.expr(value, span);
                }
                StatementKind::Break | StatementKind::Continue | StatementKind::Return(None) => {}
            }
        }
    }

    fn scoped_block(&mut self, block: &'a Block) {
        self.scopes.push(HashSet::new());
        self.block(block);
        self.scopes.pop();
    }

    fn value_block(&mut self, block: &'a ValueBlock) {
        self.scopes.push(HashSet::new());
        self.block(&block.statements);

        if let Some(tail) = &block.tail {
            self.expr(tail, block.span);
        }
        self.scopes.pop();
    }

    fn expr(&mut self, expr: &'a Expr, span: Span) {
        match expr {
            Expr::Number(_) | Expr::String(_) | Expr::Bool(_) | Expr::Identifier(_) => {}
            Expr::Unary { expr, .. } => self.expr(expr, span),
            Expr::Binary { left, right, .. }
            | Expr::Index {
                expr: left,
                index: right,
            } => {
                self.expr(left, span);
                self.expr(right, span);
            }
            Expr::Range {
                start, end, step, ..
            } => {
                self.expr(start, span);
                self.expr(end, span);

                if let Some(step) = step {
                    self.expr(step, span);
                }
            }
            Expr::Call { func, args } => {
                let declaration = if self
                    .scopes
                    .iter()
                    .rev()
                    .any(|scope| scope.contains(func.as_str()))
                {
                    None
                } else {
                    (self.call)(func, span)
                };
                // Match the checker's origin policy: omitted defaults normalize
                // positional arguments too; full-arity calls keep legacy spans.
                let normalized = args.iter().any(|arg| arg.name.is_some())
                    || declaration.is_some_and(|function| {
                        args.len() < function.params.len()
                            && function.params.iter().any(|param| param.default.is_some())
                    });

                for arg in args {
                    self.expr(&arg.value, if normalized { arg.span } else { span });
                }
            }
            Expr::Loop { body } => self.scoped_block(body),
            Expr::Block(block) => self.value_block(block),
            Expr::If { cond, then, else_ } => {
                self.expr(cond, span);
                self.value_block(then);

                if let Some(block) = else_ {
                    self.value_block(block);
                }
            }
            Expr::Array(values) => {
                for value in values {
                    self.expr(value, span);
                }
            }
        }
    }
}
