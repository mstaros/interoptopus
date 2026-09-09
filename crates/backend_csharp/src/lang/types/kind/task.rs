use crate::lang::TypeId;

/// A C# task or value-task return type for async overloads.
#[derive(Debug, Clone)]
pub struct Task {
    /// The inner type for `Task<T>`, or `None` for bare `Task` (void result).
    pub inner: Option<TypeId>,
    /// Emit a `ValueTask` surface instead of `Task`.
    pub value_task: bool,
}
