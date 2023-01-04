use core::{
    ffi::CStr,
    mem::{self, MaybeUninit},
};

#[derive(Debug)]
pub enum ParseError {
    InvalidHeader,
    InvalidMagic,
    BufferTooSmall,
    InvalidToken,
}

type Result<T> = core::result::Result<T, ParseError>;

/// Extract u32 from bytes
fn bytes_to_u32(bytes: &[mem::MaybeUninit<u8>]) -> Option<u32> {
    let maybe_uninit_bytes = bytes.get(..4)?;
    let init_bytes = unsafe { MaybeUninit::slice_assume_init_ref(maybe_uninit_bytes) };
    Some(u32::from_be_bytes(init_bytes.try_into().unwrap()))
}

/// Extract u32 from bytes + offset
fn bytes_to_u32_offset(bytes: &[mem::MaybeUninit<u8>], offset: usize) -> Option<u32> {
    let maybe_uninit_bytes = bytes.get(offset..offset + 4)?;
    let init_bytes = unsafe { MaybeUninit::slice_assume_init_ref(maybe_uninit_bytes) };
    Some(u32::from_be_bytes(init_bytes.try_into().unwrap()))
}

/// Extract u64 from bytes
fn bytes_to_u64(bytes: &[mem::MaybeUninit<u8>]) -> Option<u64> {
    let maybe_uninit_bytes = bytes.get(..8)?;
    let init_bytes = unsafe { MaybeUninit::slice_assume_init_ref(maybe_uninit_bytes) };
    Some(u64::from_be_bytes(init_bytes.try_into().unwrap()))
}

/// Extract u32 from bytes, but cast as u64
fn bytes_to_u32_as_u64(bytes: &[mem::MaybeUninit<u8>]) -> Option<u64> {
    let maybe_uninit_bytes = bytes.get(..4)?;
    let init_bytes = unsafe { MaybeUninit::slice_assume_init_ref(maybe_uninit_bytes) };
    Some(u32::from_be_bytes(init_bytes.try_into().unwrap()).into())
}

fn align4(n: usize) -> usize {
    n + (0usize.wrapping_sub(n) & 3)
}

/// DeviceTree is the class entrypoint to the Devicetree operations.
/// This code focuses only on parsing a Flattened Devicetree without using the heap.
/// The Devicetree specification can be found here:
/// https://www.devicetree.org/specifications/
#[derive(Debug)]
pub struct DeviceTree<'a> {
    data: &'a [mem::MaybeUninit<u8>], // Reference to the underlying data in memory
    header: FdtHeader,                // Parsed structure of the header
}

impl<'a> DeviceTree<'a> {
    /// Create new DeviceTree based on memory pointed to by data.
    /// Result is error if the header can't be parsed correctly.
    pub fn new(data: &'a [u8]) -> Result<Self> {
        let uninit_data = unsafe { core::mem::transmute(data) };
        FdtHeader::new(uninit_data, false).map(|header| Self { data: uninit_data, header })
    }

    /// Given a pointer to the dtb as a u64, return a DeviceTree struct.
    pub unsafe fn from_u64(ptr: u64) -> Result<Self> {
        let u8ptr = ptr as *const mem::MaybeUninit<u8>;

        // Extract the real length from the header
        let dtb_buf_for_header: &[mem::MaybeUninit<u8>] =
            unsafe { core::slice::from_raw_parts(u8ptr, mem::size_of::<FdtHeader>()) };
        let dtb_for_header = FdtHeader::new(dtb_buf_for_header, true)
            .map(|header| Self { data: dtb_buf_for_header, header })?;
        let len = dtb_for_header.header.totalsize as usize;

        // Extract the buffer for real
        let dtb_buf: &[mem::MaybeUninit<u8>] =
            unsafe { core::slice::from_raw_parts(u8ptr as *const MaybeUninit<u8>, len) };
        FdtHeader::new(dtb_buf, false).map(|header| Self { data: dtb_buf, header })
    }

    /// Return slice containing `structs` area in FDT
    fn structs(&self) -> &[mem::MaybeUninit<u8>] {
        let start = self.header.off_dt_struct as usize;
        let size: usize = self.header.size_dt_struct as usize;
        &self.data[start..(start + size)]
    }

    /// Return slice containing `strings` area in FDT (all null terminated)
    fn strings(&self) -> &'a [mem::MaybeUninit<u8>] {
        let start = self.header.off_dt_strings as usize;
        let size: usize = self.header.size_dt_strings as usize;
        &self.data[start..(start + size)]
    }

    pub fn root(&self) -> Option<Node> {
        self.node_from_index(0, 0)
    }

    pub fn children(&self, parent: &Node) -> impl Iterator<Item = Node> + '_ {
        // Start searching linearly after node.start (which points to the start of the parent)
        let mut i = parent.next_token_start;
        let child_depth = parent.depth + 1;

        core::iter::from_fn(move || {
            let child = self.node_from_index(i, child_depth)?;
            i = child.start + child.total_len;
            Some(child)
        })
    }

    /// Find the parent of child.
    pub fn parent(&self, child: &Node) -> Option<Node> {
        // Search from the root of the tree down using the depth and the bounds of the nodes
        // to find the parent.
        fn find_parent(dt: &DeviceTree, node: Node, child: &Node) -> Option<Node> {
            if !node.encloses(child) || node.depth >= child.depth {
                return None;
            }
            // At this point, we enclose the child, and we're higher up
            if node.depth + 1 < child.depth {
                // Descend into children
                for c in dt.children(&node) {
                    if let Some(found_parent) = find_parent(dt, c, child) {
                        return Some(found_parent);
                    }
                }
            }
            // Must be the parent
            return Some(node);
        }

        let root = self.root();
        return root.and_then(|n| find_parent(self, n, child));
    }

    pub fn node_name(&self, node: &Node) -> Option<&str> {
        Self::inline_str(self.structs(), node.name_start)
    }

    pub fn property(&self, node: &Node, prop_name: &str) -> Option<Property> {
        self.properties(node).filter(|p| self.property_name(p) == Some(prop_name)).next()
    }

    pub fn property_name(&self, prop: &Property) -> Option<&str> {
        Self::inline_str(self.strings(), prop.name_start)
    }

    pub fn property_value_bytes(&self, prop: &Property) -> Option<&[mem::MaybeUninit<u8>]> {
        let value_end = prop.value_start + prop.value_len;
        self.structs().get(prop.value_start..value_end)
    }

    pub fn property_value_as_u32(&self, prop: &Property) -> Option<u32> {
        let value_end = prop.value_start + prop.value_len;
        self.structs().get(prop.value_start..value_end).and_then(|bs| bytes_to_u32(bs))
    }

    pub fn property_value_as_u32_iter(&self, prop: &Property) -> impl Iterator<Item = u32> + '_ {
        let mut value_i = prop.value_start;
        let value_end = prop.value_start + prop.value_len;
        core::iter::from_fn(move || {
            if value_i >= value_end {
                return None;
            }
            let (start, end) = (value_i, value_i + 4);
            value_i = end;
            return self.structs().get(start..end).and_then(|bs| bytes_to_u32(bs));
        })
    }

    /// Return the node's #address-cells and #size-cells values as a tuple
    fn node_address_size_cells(&self, node: Option<Node>) -> (usize, usize) {
        let address_cells = node
            .and_then(|n| self.property(&n, "#address-cells"))
            .and_then(|p| self.property_value_as_u32(&p))
            .unwrap_or(2) as usize;
        let size_cells = node
            .and_then(|n| self.property(&n, "#size-cells"))
            .and_then(|p| self.property_value_as_u32(&p))
            .unwrap_or(1) as usize;
        (address_cells, size_cells)
    }

    fn consume_cells(&self, value_i: usize, num_cells: usize) -> Option<u64> {
        let bytes_fn = if num_cells == 1 { bytes_to_u32_as_u64 } else { bytes_to_u64 };
        let start = value_i;
        let end = value_i + (num_cells * 4);
        self.structs().get(start..end).and_then(|bs| bytes_fn(bs))
    }

    /// Return the reg values as u64 whether the size is 1 or 2 cells.
    /// Doesn't support > 2 cells.
    pub fn property_reg_iter(&self, node: Node) -> impl Iterator<Item = RegBlock> + '_ {
        // Get the address-cells and size-cells from the parent
        let parent = self.parent(&node);
        let (address_cells, size_cells) = self.node_address_size_cells(parent);

        // If reg doesn't exist, start and len will be zero and None will be returned from the iter
        let prop = self.property(&node, "reg");
        let (value_start, value_len) = prop.map_or((0, 0), |p| (p.value_start, p.value_len));
        let mut value_i = value_start;
        let value_end = value_start + value_len;

        core::iter::from_fn(move || {
            // size_cells may be 0 for reg (implies no len)
            if address_cells == 0 || address_cells > 2 || size_cells > 2 {
                return None;
            }

            let address_size = (address_cells * 4) as usize;
            let len_size = (size_cells * 4) as usize;

            // End if not enough bytes to parse address and len
            let remaining = value_end - value_i;
            if (address_size + len_size) > remaining {
                return None;
            }

            let addr = self.consume_cells(value_i, address_cells)?;
            value_i += address_size;

            let len = if size_cells > 0 { self.consume_cells(value_i, size_cells) } else { None };
            value_i += len_size;

            return Some(RegBlock { addr, len });
        })
    }

    /// Return the ranges values as u64 whether the size is 1 or 2 cells.
    /// Doesn't support > 2 cells.
    pub fn property_range_iter(&self, node: Node) -> impl Iterator<Item = Range> + '_ {
        // Get the address-cells and size-cells from the parent
        let parent = self.parent(&node);
        let (parent_address_cells, _) = self.node_address_size_cells(parent);
        let (address_cells, size_cells) = self.node_address_size_cells(Some(node));

        // If ranges doesn't exist, start and len will be zero and None will be returned from the iter
        let prop = self.property(&node, "ranges");
        let (value_start, value_len) = prop.map_or((0, 0), |p| (p.value_start, p.value_len));
        let mut value_i = value_start;
        let value_end = value_start + value_len;

        // If the length is zero, handle the identity range as a special case
        let is_identity = value_i == value_end;
        let mut identity_returned = false;

        core::iter::from_fn(move || {
            if is_identity {
                if !identity_returned {
                    identity_returned = true;
                    return Some(Range::Identity);
                }
                return None;
            }

            // size_cells must not be 0 for ranges
            if address_cells == 0 || size_cells == 0 || address_cells > 2 || size_cells > 2 {
                return None;
            }
            if parent_address_cells == 0 || parent_address_cells > 2 {
                return None;
            }

            let address_size = (address_cells * 4) as usize;
            let parent_address_size = (parent_address_cells * 4) as usize;
            let len_size = (size_cells * 4) as usize;

            // End if not enough bytes to parse 2x address and len
            let remaining = value_end - value_i;
            if (address_size + parent_address_size + len_size) > remaining {
                return None;
            }

            let child_bus_addr = self.consume_cells(value_i, address_cells)?;
            value_i += address_size;

            let parent_bus_addr = self.consume_cells(value_i, parent_address_cells)?;
            value_i += parent_address_size;

            let len = self.consume_cells(value_i, size_cells)?;
            value_i += len_size;

            return Some(Range::Translated(RangeMapping { child_bus_addr, parent_bus_addr, len }));
        })
    }

    /// Get the reg values, translated by ranges of the parent
    pub fn property_translated_reg_iter(
        &self,
        node: Node,
    ) -> impl Iterator<Item = TranslatedReg> + '_ {
        let mut reg_iter = self.property_reg_iter(node);
        let mut curr_reg = reg_iter.next();

        // Work on each reg element in turn
        core::iter::from_fn(move || {
            if let Some(reg) = curr_reg {
                curr_reg = reg_iter.next();

                // Walk from child to parents, translating by ranges at each step
                let mut translated_reg = reg;
                let mut curr_parent = self.parent(&node);
                while curr_parent.is_some() {
                    if let Some(parent) = curr_parent {
                        if parent.is_root() {
                            return Some(TranslatedReg::Translated(translated_reg));
                        }

                        // Find a range containing the regblock
                        let mut translated = false;
                        for range in self.property_range_iter(parent) {
                            if let Some(new_reg) = range.translate(translated_reg) {
                                translated_reg = new_reg;
                                translated = true;
                                break;
                            }
                        }

                        if !translated {
                            return Some(TranslatedReg::Unreachable);
                        }

                        curr_parent = self.parent(&parent);
                    }
                }
            }
            return None;
        })
    }

    fn property_value_contains(&self, prop: &Property, bytes_to_find: &str) -> bool {
        if let Some(uninit_value) = self.property_value_bytes(prop) {
            let init_value = unsafe { MaybeUninit::slice_assume_init_ref(uninit_value) };
            return init_value.split(|b| *b == b'\0').any(|bs| bs == bytes_to_find.as_bytes());
        }
        return false;
    }

    /// Return the node specified by the path, or None
    pub fn find_by_path(&self, path: &str) -> Option<Node> {
        fn find_subpath<'a, I>(
            dt: &DeviceTree,
            path_iter: &mut I,
            node: &Node,
            curr_path_element: Option<&str>,
        ) -> Option<Node>
        where
            I: Iterator<Item = &'a str>,
        {
            // Found the end of the path, so return the node
            let node_name = dt.node_name(node);
            if curr_path_element == node_name {
                let next_path_element = path_iter.next();
                if next_path_element.is_none() {
                    return Some(*node);
                }
                // Matching element on path, so recurse into children
                for child in dt.children(node) {
                    let found_node = find_subpath(dt, path_iter, &child, next_path_element);
                    if found_node.is_some() {
                        return found_node;
                    }
                }
            }

            return None;
        }

        // Prime the recursion with the first element of the path
        let mut path_iter = path.split_terminator('/');
        let next_path_element = path_iter.next();

        return self
            .root()
            .and_then(|node| find_subpath(self, &mut path_iter, &node, next_path_element));
    }

    /// Return the first node matching the compatible string 'comp'
    pub fn find_compatible(&'a self, comp: &'a str) -> impl Iterator<Item = Node> + '_ {
        // Iterate over all nodes.  For each node, iterate over all properties until we find a 'compatible'
        // property.  The 'compatible' property contains a list of null terminated strings.  If we find a matching
        // string, then return the node, otherwise return None.
        self.nodes().filter(|n| {
            if let Some(comp_prop) = self.property(&n, "compatible") {
                return self.property_value_contains(&comp_prop, comp);
            }
            return false;
        })
    }

    fn inline_str(bytes: &[mem::MaybeUninit<u8>], start: usize) -> Option<&str> {
        let maybe_uninit_bytes = bytes.get(start..)?;
        let init_bytes = unsafe { MaybeUninit::slice_assume_init_ref(maybe_uninit_bytes) };
        let cstr = CStr::from_bytes_until_nul(init_bytes).ok()?;
        cstr.to_str().ok()
    }

    fn node_from_index(&self, start: usize, node_depth: usize) -> Option<Node> {
        // Iterate through data, finding the start index of the beginning of the
        // FDT_BEGIN_NODE token, and the index of the end of the FDT_END_NODE token.
        let structs = self.structs();
        let mut i = start;
        let mut begin_node_ctx: Option<FdtBeginNodeContext> = None;
        let mut next_token_start = 0;
        let mut depth = node_depth;

        while i < structs.len() {
            let token = Self::parse_token(structs, i);

            match token {
                Some(FdtToken::FdtBeginNode(ctx)) => {
                    if depth == node_depth {
                        // Found the actual start of the next node
                        begin_node_ctx.replace(ctx);
                        next_token_start = i + ctx.total_len;
                    }
                    depth += 1;
                    i += ctx.total_len;
                }
                Some(FdtToken::FdtEndNode(ctx)) => {
                    depth -= 1;
                    if depth == node_depth {
                        return begin_node_ctx.map(|begin_ctx| Node {
                            start: begin_ctx.start,
                            name_start: begin_ctx.name_start,
                            next_token_start,
                            total_len: (ctx.start + ctx.total_len) - begin_ctx.start,
                            depth: node_depth,
                        });
                    }
                    i += ctx.total_len;
                }
                Some(FdtToken::FdtProp(ctx)) => {
                    i += ctx.total_len;
                }
                Some(FdtToken::FdtNop(ctx) | FdtToken::FdtEnd(ctx)) => {
                    i += ctx.total_len;
                }
                None => return None, // Shouldn't get here normally, so just None
            }
        }
        // Node returned at FDT_END_NODE
        None
    }

    /// Linearly iterate over the nodes in the order they occur in the flattened device tree
    pub fn nodes(&self) -> impl Iterator<Item = Node> + '_ {
        let structs = self.structs();
        let mut i = 0;
        let mut depth = 0;

        // On each iteration, i should be at or before the next node token we expect.
        // This is achieved by setting it to next_token_start when the end node token is found.
        // next_token_start is set when the first begin node token of the iteration is found, and
        // points to the next token after that begin node token.

        core::iter::from_fn(move || {
            let mut node_depth = 0;
            let mut next_token_start = 0;
            let mut begin_node_ctx: Option<FdtBeginNodeContext> = None;

            while i < structs.len() {
                let token = Self::parse_token(structs, i);
                if let Some(token) = token {
                    match token {
                        FdtToken::FdtBeginNode(ctx) => {
                            if begin_node_ctx.is_none() {
                                begin_node_ctx.replace(ctx);
                                node_depth = depth;
                                next_token_start = i + ctx.total_len;
                            }
                            depth += 1;
                            i += ctx.total_len;
                        }

                        FdtToken::FdtEndNode(ctx) => {
                            if begin_node_ctx.is_some() && (depth - 1) == node_depth {
                                // Reset i for the next node iteration
                                i = next_token_start;
                                let new_node = begin_node_ctx.take().map(|begin_ctx| Node {
                                    start: begin_ctx.start,
                                    name_start: begin_ctx.name_start,
                                    next_token_start,
                                    total_len: (ctx.start + ctx.total_len) - begin_ctx.start,
                                    depth: node_depth,
                                });
                                return new_node;
                            }

                            depth -= 1;
                            i += ctx.total_len;
                        }
                        FdtToken::FdtProp(ctx) => {
                            i += ctx.total_len;
                        }
                        FdtToken::FdtNop(ctx) | FdtToken::FdtEnd(ctx) => {
                            i += ctx.total_len;
                        }
                    }
                } else {
                    return None;
                }
            }
            None
        })
    }

    /// Linearly iterate over the properties of a node in the order they occur in the flattened device tree
    fn properties(&self, node: &Node) -> impl Iterator<Item = Property> + '_ {
        let structs = self.structs();
        let end_i = node.start + node.total_len;
        let mut i = node.next_token_start;

        core::iter::from_fn(move || {
            while i < end_i {
                let token = Self::parse_token(structs, i);

                // Node properties come before any children
                match token {
                    Some(FdtToken::FdtProp(ctx)) => {
                        i += ctx.total_len;
                        return Some(Property {
                            start: ctx.start,
                            name_start: ctx.name_start,
                            value_start: ctx.value_start,
                            value_len: ctx.value_len,
                            total_len: ctx.total_len,
                        });
                    }
                    Some(FdtToken::FdtNop(ctx)) => {
                        i += ctx.total_len;
                    }
                    _ => return None,
                }
            }
            None
        })
    }

    fn parse_token(structs: &[mem::MaybeUninit<u8>], i: usize) -> Option<FdtToken> {
        let token = structs.get(i..).and_then(|bs| bytes_to_u32(bs));

        match token {
            Some(0x1) => {
                // Null terminated string follow token
                let str_size = structs
                    .get((i + 4)..)
                    .and_then(|bs| unsafe {
                        MaybeUninit::slice_assume_init_ref(bs).iter().position(|&b| b == 0)
                    })
                    .map(|sz| align4(sz + 1))
                    .unwrap_or(0);
                return Some(FdtToken::FdtBeginNode(FdtBeginNodeContext {
                    start: i,
                    name_start: i + 4,
                    total_len: 4 + str_size,
                }));
            }
            Some(0x2) => {
                return Some(FdtToken::FdtEndNode(FdtTokenContext { start: i, total_len: 4 }));
            }
            Some(0x3) => {
                let len = structs.get((i + 4)..).and_then(|bs| bytes_to_u32(bs)).unwrap_or(0);
                let nameoff = structs.get((i + 8)..).and_then(|bs| bytes_to_u32(bs)).unwrap_or(0);
                return Some(FdtToken::FdtProp(FdtPropContext {
                    start: i,
                    name_start: nameoff as usize,
                    value_start: i + 12,
                    value_len: len as usize,
                    total_len: 12 + align4(len as usize),
                }));
            }
            Some(0x4) => {
                return Some(FdtToken::FdtNop(FdtTokenContext { start: i, total_len: 4 }));
            }
            Some(0x9) => {
                return Some(FdtToken::FdtEnd(FdtTokenContext { start: i, total_len: 4 }));
            }
            _ => {
                return None;
            }
        }
    }
}

/// Flattened Devicetree header structure, as documented in the spec
#[derive(Debug)]
#[allow(dead_code)]
struct FdtHeader {
    magic: u32,
    totalsize: u32,
    off_dt_struct: u32,
    off_dt_strings: u32,
    off_mem_rsvmap: u32,
    version: u32,
    last_comp_version: u32,
    boot_cpuid_phys: u32,
    size_dt_strings: u32,
    size_dt_struct: u32,
}

impl FdtHeader {
    /// Read FdtHeader from the stream, returning it in a Result if it passes validation,
    /// otherwise returns an error.
    /// Set ignore_size to true if you're only loading a portion of the buffer
    /// (e.g. in order to work out the size of the buffer before casting to a slice).
    fn new(data: &[mem::MaybeUninit<u8>], ignore_size: bool) -> Result<Self> {
        fn new_header(data: &[mem::MaybeUninit<u8>]) -> Option<FdtHeader> {
            Some(FdtHeader {
                magic: bytes_to_u32_offset(data, 0)?,
                totalsize: bytes_to_u32_offset(data, 4)?,
                off_dt_struct: bytes_to_u32_offset(data, 8)?,
                off_dt_strings: bytes_to_u32_offset(data, 12)?,
                off_mem_rsvmap: bytes_to_u32_offset(data, 16)?,
                version: bytes_to_u32_offset(data, 20)?,
                last_comp_version: bytes_to_u32_offset(data, 24)?,
                boot_cpuid_phys: bytes_to_u32_offset(data, 28)?,
                size_dt_strings: bytes_to_u32_offset(data, 32)?,
                size_dt_struct: bytes_to_u32_offset(data, 36)?,
            })
        }

        let len = data.len() as u32;
        new_header(data)
            .ok_or(ParseError::InvalidHeader)
            .and_then(|h| (h.magic == 0xd00dfeed).then_some(h).ok_or(ParseError::InvalidMagic))
            .and_then(|h| {
                (len == h.totalsize || ignore_size).then_some(h).ok_or(ParseError::BufferTooSmall)
            })
    }
}

/// Token represents one of 5 tokens in the FDT specification.  The names and IDs correspond
/// to those in the specification.
#[derive(Debug, Copy, Clone)]
enum FdtToken {
    FdtBeginNode(FdtBeginNodeContext), // Start of a new node
    FdtEndNode(FdtTokenContext),       // End of current node
    FdtProp(FdtPropContext),           // New property in current node
    FdtNop(FdtTokenContext),           // To be ignored
    FdtEnd(FdtTokenContext),           // End of FDT structure
}

#[derive(Debug, Copy, Clone)]
struct FdtBeginNodeContext {
    start: usize,      // Start of token in buffer
    total_len: usize,  // Number of bytes for token, name and alignment
    name_start: usize, // Start of node name in sturcts buffer
}

#[derive(Debug, Copy, Clone)]
struct FdtPropContext {
    start: usize,       // Start of token in buffer
    total_len: usize,   // Number of bytes for token, len, nameoff, value and alignment
    name_start: usize,  // Start of node name in strings buffer
    value_start: usize, // Start of property value
    value_len: usize,   // Size of property value
}

#[derive(Debug, Copy, Clone)]
struct FdtTokenContext {
    start: usize,     // Start of token in buffer
    total_len: usize, // Number of bytes for token
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub struct Node {
    start: usize,            // Start index in structs of node (Start of FDT_BEGIN_NODE)
    name_start: usize,       // Start of node name in structs buffer
    next_token_start: usize, // Start index of token after FDT_BEGIN_NODE
    total_len: usize,        // Total length of node
    depth: usize,            // Depth of node, with 0 being the root
}

impl Node {
    fn encloses(&self, child: &Node) -> bool {
        let parent_starts_before_child = self.start <= child.start;
        let parent_ends_after_child =
            (self.start + self.total_len) >= (child.start + child.total_len);
        parent_starts_before_child && parent_ends_after_child
    }

    fn is_root(&self) -> bool {
        self.depth == 0
    }

    pub fn depth(&self) -> usize {
        self.depth
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub struct Property {
    start: usize,       // Start index in structs of node (Start of FDT_BEGIN_NODE)
    name_start: usize,  // Start of node name in strings buffer
    value_start: usize, // Start index of property value
    value_len: usize,   // Size of property value
    total_len: usize,   // Total length of property
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub struct RegBlock {
    pub addr: u64,
    pub len: Option<u64>,
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum TranslatedReg {
    Translated(RegBlock),
    Unreachable,
}

impl TranslatedReg {
    pub fn regblock(&self) -> Option<RegBlock> {
        match self {
            TranslatedReg::Translated(regblock) => Some(*regblock),
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub struct RangeMapping {
    pub child_bus_addr: u64,
    pub parent_bus_addr: u64,
    pub len: u64,
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum Range {
    Identity,
    Translated(RangeMapping),
}

impl Range {
    /// Attempt to translate the given RegBlock.  If it can't be mapped, return None.
    fn translate(&self, r: RegBlock) -> Option<RegBlock> {
        match self {
            Range::Identity => Some(r),
            Range::Translated(map) => {
                if r.addr >= map.child_bus_addr && r.addr < map.child_bus_addr + map.len {
                    let addr = r.addr - map.child_bus_addr + map.parent_bus_addr;
                    return Some(RegBlock { addr, len: r.len });
                }
                return None;
            }
        }
    }
}
