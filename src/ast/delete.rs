use crate::ast::*;

#[derive(Debug, PartialEq, Clone)]
/// A builder for a `DELETE` statement.
pub struct Delete<'a> {
    pub(crate) table: Table<'a>,
    pub(crate) conditions: Option<ConditionTree<'a>>,
}

impl<'a> From<Delete<'a>> for Query<'a> {
    fn from(delete: Delete<'a>) -> Self {
        Query::Delete(Box::new(delete))
    }
}

impl<'a> Delete<'a> {
    /// Creates a new `DELETE` statement for the given table.
    ///
    /// ```rust
    /// # use quaint::{ast::*, visitor::{Visitor, Sqlite}};
    /// # fn main() -> Result<(), quaint::error::Error> {
    /// let query = Delete::from_table("users");
    /// let (sql, _) = Sqlite::build(query)?;
    ///
    /// assert_eq!("DELETE FROM `users`", sql);
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_table<T>(table: T) -> Self
    where
        T: Into<Table<'a>>,
    {
        Self {
            table: table.into(),
            conditions: None,
        }
    }

    /// Adds `WHERE` conditions to the query. See
    /// [Comparable](trait.Comparable.html#required-methods) for more examples.
    ///
    /// ```rust
    /// # use quaint::{ast::*, visitor::{Visitor, Sqlite}};
    /// # fn main() -> Result<(), quaint::error::Error> {
    /// let query = Delete::from_table("users").so_that("bar".equals(false));
    /// let (sql, params) = Sqlite::build(query)?;
    ///
    /// assert_eq!("DELETE FROM `users` WHERE `bar` = ?", sql);
    /// assert_eq!(vec![Value::boolean(false)], params);
    /// # Ok(())
    /// # }
    /// ```
    pub fn so_that<T>(mut self, conditions: T) -> Self
    where
        T: Into<ConditionTree<'a>>,
    {
        self.conditions = Some(conditions.into());
        self
    }

    /// A list of item names in the query, skipping the anonymous values or
    /// columns.
    pub(crate) fn named_selection(&self) -> Vec<String> {
        // TODO Implement returning first
        vec![]
    }
}
