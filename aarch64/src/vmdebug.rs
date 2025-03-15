///! Debug tools for VM code

#[cfg(not(test))]
use port::println;

use crate::vm::{Entry, Level, RootPageTable, RootPageTableType, Table};

#[derive(Clone, Copy, Debug, PartialEq)]
struct PteIndices {
    pgtype: RootPageTableType,
    l0: Option<usize>,
    l1: Option<usize>,
    l2: Option<usize>,
    l3: Option<usize>,
}

impl PteIndices {
    #[cfg(test)]
    fn new(
        pgtype: RootPageTableType,
        l0: Option<usize>,
        l1: Option<usize>,
        l2: Option<usize>,
        l3: Option<usize>,
    ) -> Self {
        Self { pgtype, l0, l1, l2, l3 }
    }

    fn none(pgtype: RootPageTableType) -> Self {
        Self { pgtype, l0: None, l1: None, l2: None, l3: None }
    }

    fn with_next_index(&self, i: usize) -> Option<Self> {
        if self.l0.is_none() {
            Some(Self { pgtype: self.pgtype, l0: Some(i), l1: None, l2: None, l3: None })
        } else if self.l1.is_none() {
            Some(Self { pgtype: self.pgtype, l0: self.l0, l1: Some(i), l2: None, l3: None })
        } else if self.l2.is_none() {
            Some(Self { pgtype: self.pgtype, l0: self.l0, l1: self.l1, l2: Some(i), l3: None })
        } else if self.l3.is_none() {
            Some(Self { pgtype: self.pgtype, l0: self.l0, l1: self.l1, l2: self.l2, l3: Some(i) })
        } else {
            None
        }
    }

    fn with_last_index(&self, i: usize) -> Option<Self> {
        if self.l0.is_none() {
            None
        } else if self.l1.is_none() {
            Some(Self { pgtype: self.pgtype, l0: Some(i), l1: None, l2: None, l3: None })
        } else if self.l2.is_none() {
            Some(Self { pgtype: self.pgtype, l0: self.l0, l1: Some(i), l2: None, l3: None })
        } else if self.l3.is_none() {
            Some(Self { pgtype: self.pgtype, l0: self.l0, l1: self.l1, l2: Some(i), l3: None })
        } else {
            Some(Self { pgtype: self.pgtype, l0: self.l0, l1: self.l1, l2: self.l2, l3: Some(i) })
        }
    }

    fn last_index(&self) -> Option<usize> {
        if let Some(i) = self.l3 {
            Some(i)
        } else if let Some(i) = self.l2 {
            Some(i)
        } else if let Some(i) = self.l1 {
            Some(i)
        } else if let Some(i) = self.l0 {
            Some(i)
        } else {
            None
        }
    }

    fn to_va(&self) -> usize {
        let mut va = match self.pgtype {
            RootPageTableType::Kernel => 0xffff_0000_0000_0000,
            RootPageTableType::User => 0x0000_0000_0000_0000,
        };

        va |= if let Some(i) = self.l0 { i << 39 } else { 0 };
        va |= if let Some(i) = self.l1 { i << 30 } else { 0 };
        va |= if let Some(i) = self.l2 { i << 21 } else { 0 };
        va |= if let Some(i) = self.l3 { i << 12 } else { 0 };

        va
    }
}

/// Return recursive virtual addresses for the current kernel or user page tables.
/// This depends on the recursive entry of root page tables to have been set up correctly.
fn recursive_root_page_table_va(pgtype: RootPageTableType) -> usize {
    match pgtype {
        RootPageTableType::User => 0x0000_ffff_ffff_f000,
        RootPageTableType::Kernel => 0xffff_ffff_ffff_f000,
    }
}

/// Return the current kernel or user page table.
/// This depends on the recursive entry of root page tables to have been set up correctly.
fn recursive_root_page_table(pgtype: RootPageTableType) -> &'static mut RootPageTable {
    let ptr = recursive_root_page_table_va(pgtype) as *mut RootPageTable;
    unsafe { &mut *ptr }
}

/// Recursively write out all the tables and all its children
pub fn print_recursive_tables(pgtype: RootPageTableType) {
    let root_page_table = recursive_root_page_table(pgtype);
    println!("Root va:{:018p}", root_page_table);
    print_table_at_level(
        root_page_table,
        Level::Level0,
        recursive_root_page_table_va(pgtype),
        pgtype,
        PteIndices::none(pgtype),
    );
}

/// Recursively write out the table and all its children
fn print_table_at_level(
    page_table: &Table,
    level: Level,
    table_va: usize,
    pgtype: RootPageTableType,
    pte_indices: PteIndices,
) {
    let indent = 2 + level.depth() * 2;
    println!("{:indent$}Table {:?} va:{:018p}", "", level, page_table);

    for i in 0..512 {
        let pte = page_table.entries[i];
        if !pte.valid() {
            continue;
        }

        if !pte.is_table(level) {
            if let Some(pte_indices) = pte_indices.with_last_index(i) {
                print_pte_page(indent, pte_indices, pte);
            }
        } else if i != 511 {
            // Recurse into child table (unless it's the recursive index)
            let child_table_va = match pgtype {
                RootPageTableType::User => ((table_va << 9) | (i << 12)) & 0x0000_ffff_ffff_ffff,
                RootPageTableType::Kernel => (table_va << 9) | (i << 12),
            };
            print_pte_table(indent, i, pte, child_table_va);

            if let Some(next_level_pte_indices) = pte_indices.with_next_index(i) {
                let next_nevel = level.next().unwrap();
                let child_table = unsafe { &*(child_table_va as *const RootPageTable) };
                print_table_at_level(
                    child_table,
                    next_nevel,
                    child_table_va,
                    pgtype,
                    next_level_pte_indices,
                );
            }
        }
    }
}

/// Helper to print out page PTE
fn print_pte_page(indent: usize, pte_indices: PteIndices, pte: Entry) {
    println!(
        "{:indent$}[{:03}] Entry va:{:#018x} -> {:?} (pte:{:#016x})",
        "",
        pte_indices.last_index().unwrap_or(0),
        pte_indices.to_va(),
        pte,
        pte.0,
    );
}

/// Helper to print out table PTE
fn print_pte_table(indent: usize, i: usize, pte: Entry, table_va: usize) {
    println!(
        "{:indent$}[{:03}] Table va:{:#018x} {:?} (pte:{:#016x})",
        "", i, table_va, pte, pte.0,
    );
}

/// Returns a tuple of page table indices for the given virtual address
#[cfg(test)]
pub fn va_indices(va: usize) -> (usize, usize, usize, usize) {
    use crate::vm::va_index;

    (
        va_index(va, Level::Level0),
        va_index(va, Level::Level1),
        va_index(va, Level::Level2),
        va_index(va, Level::Level3),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pte_indices() {
        let p = PteIndices::none(RootPageTableType::User);
        assert_eq!(p, PteIndices::none(RootPageTableType::User));

        let p = p.with_next_index(1).unwrap();
        assert_eq!(p, PteIndices::new(RootPageTableType::User, Some(1), None, None, None));

        let p = p.with_next_index(2).unwrap();
        assert_eq!(p, PteIndices::new(RootPageTableType::User, Some(1), Some(2), None, None));

        let p = p.with_next_index(3).unwrap();
        assert_eq!(p, PteIndices::new(RootPageTableType::User, Some(1), Some(2), Some(3), None));

        let p = p.with_next_index(4).unwrap();
        assert_eq!(p, PteIndices::new(RootPageTableType::User, Some(1), Some(2), Some(3), Some(4)));

        let p = PteIndices::new(RootPageTableType::Kernel, Some(1), Some(2), None, None);
        let p = p.with_last_index(33).unwrap();
        assert_eq!(p, PteIndices::new(RootPageTableType::Kernel, Some(1), Some(33), None, None));
        assert_eq!(p.last_index(), Some(33));

        let p = PteIndices::new(RootPageTableType::Kernel, Some(1), Some(2), Some(3), Some(4));
        let p = p.with_last_index(100).unwrap();
        assert_eq!(
            p,
            PteIndices::new(RootPageTableType::Kernel, Some(1), Some(2), Some(3), Some(100))
        );
        assert_eq!(p.last_index(), Some(100));

        let p = PteIndices::new(RootPageTableType::Kernel, Some(15), Some(0), Some(400), Some(4));
        assert_eq!(va_indices(p.to_va()), (15, 0, 400, 4));

        let p = PteIndices::new(RootPageTableType::User, Some(0), Some(10), Some(40), Some(23));
        assert_eq!(va_indices(p.to_va()), (0, 10, 40, 23));
    }
}
