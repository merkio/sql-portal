//! Pagination of query results: which page is asked for and which exists.

/// A page the caller asks for. `number` is 1-based.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRequest {
    pub number: usize,
    pub size: usize,
}

/// The page that is actually served, after clamping to the pages that exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageInfo {
    pub number: usize,
    pub size: usize,
    /// Rows matching the query, across all pages.
    pub total: u64,
    /// Number of pages; at least one, even for an empty result.
    pub pages: usize,
}

impl PageInfo {
    /// Clamps the requested page to the pages that exist. A page number below
    /// one is treated as the first page, one above the last as the last page.
    pub fn plan(request: PageRequest, total: u64) -> PageInfo {
        let size = request.size.max(1);
        let pages = usize::try_from(total.div_ceil(size as u64))
            .unwrap_or(usize::MAX)
            .max(1);
        PageInfo {
            number: request.number.clamp(1, pages),
            size,
            total,
            pages,
        }
    }

    /// Rows before this page.
    pub fn offset(&self) -> u64 {
        ((self.number - 1) * self.size) as u64
    }

    /// Position of the first row on this page, 1-based; 0 when there are none.
    pub fn first_row(&self) -> u64 {
        if self.total == 0 {
            0
        } else {
            self.offset() + 1
        }
    }

    /// Position of the last row on this page, 1-based.
    pub fn last_row(&self) -> u64 {
        (self.offset() + self.size as u64).min(self.total)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(number: usize, size: usize, total: u64) -> PageInfo {
        PageInfo::plan(PageRequest { number, size }, total)
    }

    #[test]
    fn splits_rows_into_pages() {
        let p = plan(2, 25, 60);
        assert_eq!((p.number, p.pages, p.offset()), (2, 3, 25));
        assert_eq!((p.first_row(), p.last_row()), (26, 50));
        let last = plan(3, 25, 60);
        assert_eq!((last.first_row(), last.last_row()), (51, 60));
    }

    #[test]
    fn clamps_out_of_range_pages() {
        assert_eq!(plan(0, 25, 60).number, 1);
        assert_eq!(plan(99, 25, 60).number, 3);
    }

    #[test]
    fn an_empty_result_has_one_empty_page() {
        let p = plan(4, 25, 0);
        assert_eq!((p.number, p.pages), (1, 1));
        assert_eq!((p.first_row(), p.last_row()), (0, 0));
    }

    #[test]
    fn a_zero_size_is_treated_as_one_row_per_page() {
        let p = plan(1, 0, 3);
        assert_eq!((p.size, p.pages), (1, 3));
    }
}
