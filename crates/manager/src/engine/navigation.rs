//! Navigation state of a device: stack of module pages above the home page
//! and first visible row of the displayed page.

use hubergamof_module_api::{ModuleInstanceId, PageId};

/// A module page on the navigation stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PageRef {
    pub(crate) module: ModuleInstanceId,
    pub(crate) page: PageId,
}

/// Pages opened above the home page, and the visible row of the top one.
/// Every page change shows the first row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Navigation {
    stack: Vec<PageRef>,
    row: u32,
}

impl Navigation {
    /// Displayed module page, `None` on the home page.
    pub(crate) fn current(&self) -> Option<&PageRef> {
        self.stack.last()
    }

    /// Number of pages above the home page.
    pub(crate) fn depth(&self) -> usize {
        self.stack.len()
    }

    /// First visible row.
    pub(crate) fn row(&self) -> u32 {
        self.row
    }

    pub(crate) fn set_row(&mut self, row: u32) {
        self.row = row;
    }

    pub(crate) fn open(&mut self, module: ModuleInstanceId, page: PageId) {
        self.stack.push(PageRef { module, page });
        self.row = 0;
    }

    /// Goes to the previous page; does nothing on the home page.
    pub(crate) fn back(&mut self) {
        if self.stack.pop().is_some() {
            self.row = 0;
        }
    }

    pub(crate) fn home(&mut self) {
        if !self.stack.is_empty() {
            self.stack.clear();
            self.row = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_and_home_unwind_the_stack_and_reset_the_row() {
        let mut nav = Navigation::default();
        nav.set_row(1);
        nav.back();
        assert_eq!((nav.depth(), nav.row()), (0, 1));

        nav.open("docker".into(), "containers".into());
        assert_eq!(nav.row(), 0);
        nav.set_row(2);
        nav.open("docker".into(), "logs".into());
        assert_eq!(nav.current().unwrap().page, PageId::from("logs"));
        nav.back();
        assert_eq!(nav.current().unwrap().page, PageId::from("containers"));
        assert_eq!(nav.row(), 0);
        nav.open("docker".into(), "logs".into());
        nav.home();
        assert_eq!((nav.depth(), nav.current()), (0, None));
    }
}
