//! xdelta3's FGK secondary compressor: adaptive Huffman coding (Knuth's "Dynamic Huffman
//! Coding"), one tree per kind of section (data, instructions, addresses) kept from one
//! window to the next. A port of xdelta3-fgk.h, its node and block pointers as indexes.
//!
//! xdelta3's encoder runs every section of 10 bytes or more through its tree, and keeps the
//! compressed result only when it saves space: a section it left uncompressed still changed
//! its tree. xdelta3's own decoder does not follow (it fails on such patches); this one
//! replays those sections through its tree ([`Fgk::replay`]), as the encoder did.

use super::{err, Result};

const ALPHABET_SIZE: usize = 256;
const TOTAL_NODES: usize = 2 * ALPHABET_SIZE - 1;
const TOTAL_BLOCKS: usize = 2 * TOTAL_NODES;

/// The smallest section xdelta3's encoder compresses (`SECONDARY_MIN_INPUT`).
pub(super) const MIN_INPUT: usize = 10;

#[derive(Clone, Copy, Default)]
struct Node {
    weight: u32,
    parent: Option<usize>,
    left_child: Option<usize>,
    right_child: Option<usize>,
    left: Option<usize>,
    right: Option<usize>,
    block: Option<usize>,
}

#[derive(Clone, Copy, Default)]
struct Block {
    leader: Option<usize>,
    next_free: Option<usize>,
}

fn broken<T>() -> Result<T> {
    err("the patch is damaged (FGK: an inconsistent code tree)")
}

/// The node an optional pointer points to, or the error for a damaged patch.
fn at(p: Option<usize>) -> Result<usize> {
    p.map_or_else(broken, Ok)
}

pub(super) struct Fgk {
    nodes: Vec<Node>,
    blocks: Vec<Block>,
    free_block: Option<usize>,
    zero_freq_count: usize,
    zero_freq_exp: usize,
    zero_freq_rem: usize,
    coded_bits: Vec<u8>,
    decode_ptr: usize,
    remaining_zeros: Option<usize>,
    root: usize,
    free_node: usize,
}

impl Fgk {
    pub(super) fn new() -> Self {
        let mut h = Fgk {
            nodes: vec![Node::default(); TOTAL_NODES],
            blocks: vec![Block::default(); TOTAL_BLOCKS],
            free_block: Some(0),
            zero_freq_count: ALPHABET_SIZE + 2,
            zero_freq_exp: 0,
            zero_freq_rem: 0,
            coded_bits: Vec::with_capacity(ALPHABET_SIZE),
            decode_ptr: 0,
            remaining_zeros: Some(0),
            root: 0,
            free_node: ALPHABET_SIZE,
        };
        h.factor_remaining();
        h.factor_remaining();
        for (i, b) in h.blocks.iter_mut().enumerate() {
            b.next_free = (i + 1 < TOTAL_BLOCKS).then_some(i + 1);
        }
        for i in 0..ALPHABET_SIZE {
            h.nodes[i] = Node {
                right_child: (i < ALPHABET_SIZE - 1).then_some(i + 1),
                left_child: i.checked_sub(1),
                ..Node::default()
            };
        }
        h
    }

    fn factor_remaining(&mut self) {
        self.zero_freq_count -= 1;
        let mut i = self.zero_freq_count;
        self.zero_freq_exp = 0;
        while i > 1 {
            self.zero_freq_exp += 1;
            i >>= 1;
        }
        self.zero_freq_rem = self.zero_freq_count - (1 << self.zero_freq_exp);
    }

    fn make_block(&mut self, lead: usize) -> Result<usize> {
        let b = at(self.free_block)?;
        self.free_block = self.blocks[b].next_free;
        self.blocks[b].leader = Some(lead);
        Ok(b)
    }

    fn free_block(&mut self, b: Option<usize>) -> Result<()> {
        let b = at(b)?;
        self.blocks[b].next_free = self.free_block;
        self.free_block = Some(b);
        Ok(())
    }

    fn weight(&self, p: Option<usize>) -> Result<u32> {
        Ok(self.nodes[at(p)?].weight)
    }

    /// The parent's child slot holding `node`: right or left.
    fn is_right_child(&self, node: usize) -> Result<bool> {
        let parent = at(self.nodes[node].parent)?;
        Ok(self.nodes[parent].right_child == Some(node))
    }

    fn set_child(&mut self, parent: usize, right: bool, child: usize) {
        if right {
            self.nodes[parent].right_child = Some(child);
        } else {
            self.nodes[parent].left_child = Some(child);
        }
    }

    fn update_tree(&mut self, n: usize) -> Result<()> {
        let mut incr = if self.nodes[n].weight == 0 {
            self.increase_zero_weight(n)?
        } else {
            n
        };
        while incr != self.root {
            self.move_right(incr)?;
            self.promote(incr)?;
            self.nodes[incr].weight += 1;
            incr = at(self.nodes[incr].parent)?;
        }
        self.nodes[self.root].weight += 1;
        Ok(())
    }

    fn move_right(&mut self, fwd: usize) -> Result<()> {
        let back = at(self.blocks[at(self.nodes[fwd].block)?].leader)?;
        if fwd == back || self.nodes[fwd].parent == Some(back) || self.nodes[fwd].weight == 0 {
            return Ok(());
        }
        let back_right = at(self.nodes[back].right)?;
        self.nodes[back_right].left = Some(fwd);
        if let Some(l) = self.nodes[fwd].left {
            self.nodes[l].right = Some(back);
        }
        let tmp = self.nodes[fwd].right;
        self.nodes[fwd].right = self.nodes[back].right;
        if tmp == Some(back) {
            self.nodes[back].right = Some(fwd);
        } else {
            self.nodes[at(tmp)?].left = Some(back);
            self.nodes[back].right = tmp;
        }
        let tmp = self.nodes[back].left;
        self.nodes[back].left = self.nodes[fwd].left;
        if tmp == Some(fwd) {
            self.nodes[fwd].left = Some(back);
        } else {
            self.nodes[at(tmp)?].right = Some(fwd);
            self.nodes[fwd].left = tmp;
        }

        let (fwd_parent, fwd_right) = (at(self.nodes[fwd].parent)?, self.is_right_child(fwd)?);
        let (back_parent, back_right) = (at(self.nodes[back].parent)?, self.is_right_child(back)?);
        self.nodes[fwd].parent = Some(back_parent);
        self.nodes[back].parent = Some(fwd_parent);
        self.set_child(fwd_parent, fwd_right, back);
        self.set_child(back_parent, back_right, fwd);

        let b = at(self.nodes[fwd].block)?;
        self.blocks[b].leader = Some(fwd);
        Ok(())
    }

    fn promote(&mut self, node: usize) -> Result<()> {
        let my_right = self.nodes[node].right;
        let my_left = self.nodes[node].left;
        let cur = self.nodes[node].block;
        let weight = self.nodes[node].weight;
        if weight == 0 {
            return Ok(());
        }
        let right_is_next = |h: &Self| -> Result<bool> {
            Ok(weight == h.weight(my_right)?.wrapping_sub(1) && my_right != Some(h.root))
        };
        // A node whose children are the zeros and the leaf just added.
        if my_left == self.nodes[node].right_child {
            if let Some(lc) = self.nodes[node].left_child {
                if self.nodes[lc].weight == 0 {
                    if right_is_next(self)? {
                        self.free_block(cur)?;
                        let b = self.nodes[at(my_right)?].block;
                        self.nodes[node].block = b;
                        self.nodes[at(my_left)?].block = b;
                    }
                    return Ok(());
                }
            }
        }
        if my_left == self.remaining_zeros {
            return Ok(());
        }
        let left = at(my_left)?;
        if self.nodes[left].block == cur {
            self.blocks[at(cur)?].leader = Some(left);
        } else {
            self.free_block(cur)?;
        }
        self.nodes[node].block = if right_is_next(self)? {
            self.nodes[at(my_right)?].block
        } else {
            Some(self.make_block(node)?)
        };
        Ok(())
    }

    fn increase_zero_weight(&mut self, n: usize) -> Result<usize> {
        let this = n;
        if self.zero_freq_count == 1 {
            self.nodes[this].right_child = None;
            let right = self.nodes[this].right;
            self.nodes[this].block = if self.weight(right)? == 1 {
                self.nodes[at(right)?].block
            } else {
                Some(self.make_block(this)?)
            };
            self.remaining_zeros = None;
            return Ok(this);
        }
        let zero = at(self.remaining_zeros)?;
        if self.free_node >= TOTAL_NODES {
            return broken();
        }
        let new = self.free_node;
        self.free_node += 1;
        self.nodes[new].parent = self.nodes[zero].parent;
        self.nodes[new].right = self.nodes[zero].right;
        self.nodes[new].weight = 0;
        self.nodes[new].right_child = Some(this);
        self.nodes[new].left = Some(this);
        if zero == self.root {
            self.root = new;
            self.nodes[this].block = Some(self.make_block(this)?);
            self.nodes[new].block = Some(self.make_block(new)?);
        } else {
            let right = at(self.nodes[new].right)?;
            self.nodes[right].left = Some(new);
            let parent = at(self.nodes[zero].parent)?;
            let on_right = self.nodes[parent].right_child == Some(zero);
            self.set_child(parent, on_right, new);
            self.nodes[new].block = if self.nodes[right].weight == 1 {
                self.nodes[right].block
            } else {
                Some(self.make_block(new)?)
            };
            self.nodes[this].block = self.nodes[new].block;
        }
        self.eliminate_zero(this)?;
        let zeros = at(self.remaining_zeros)?;
        self.nodes[new].left_child = Some(zeros);
        self.nodes[this].right = Some(new);
        self.nodes[this].left = Some(zeros);
        self.nodes[this].parent = Some(new);
        self.nodes[this].left_child = None;
        self.nodes[this].right_child = None;
        self.nodes[zeros].parent = Some(new);
        self.nodes[zeros].right = Some(this);
        Ok(this)
    }

    fn eliminate_zero(&mut self, node: usize) -> Result<()> {
        if self.zero_freq_count == 1 {
            return Ok(());
        }
        self.factor_remaining();
        match (self.nodes[node].left_child, self.nodes[node].right_child) {
            (None, _) => {
                let next = self.nodes[at(self.remaining_zeros)?].right_child;
                self.remaining_zeros = next;
                self.nodes[at(next)?].left_child = None;
            }
            (Some(l), None) => self.nodes[l].right_child = None,
            (Some(l), Some(r)) => {
                self.nodes[r].left_child = Some(l);
                self.nodes[l].right_child = Some(r);
            }
        }
        Ok(())
    }

    /// One bit of a code: whether it completes a symbol.
    fn decode_bit(&mut self, b: bool) -> Result<bool> {
        if self.nodes[self.decode_ptr].weight == 0 {
            // A symbol not seen yet: its index among those, in so many bits.
            let required = self.zero_freq_exp + usize::from(self.zero_freq_rem != 0);
            if self.coded_bits.len() >= ALPHABET_SIZE {
                return broken();
            }
            self.coded_bits.push(u8::from(b));
            return Ok(self.coded_bits.len() >= required);
        }
        let node = self.nodes[self.decode_ptr];
        self.decode_ptr = at(if b { node.right_child } else { node.left_child })?;
        let next = self.nodes[self.decode_ptr];
        Ok(next.left_child.is_none() && (next.weight != 0 || self.zero_freq_count == 1))
    }

    fn nth_zero(&self, mut n: usize) -> Result<usize> {
        let mut ret = at(self.remaining_zeros)?;
        while n != 0 {
            match self.nodes[ret].right_child {
                Some(r) => ret = r,
                None => break,
            }
            n -= 1;
        }
        Ok(ret)
    }

    fn decode_data(&mut self) -> Result<u8> {
        let mut elt = self.decode_ptr;
        if self.nodes[self.decode_ptr].weight == 0 {
            let n = self
                .coded_bits
                .iter()
                .fold(0usize, |n, &b| (n << 1) | usize::from(b));
            elt = self.nth_zero(n)?;
        }
        if elt >= ALPHABET_SIZE {
            return broken();
        }
        self.coded_bits.clear();
        self.update_tree(elt)?;
        self.decode_ptr = self.root;
        Ok(elt as u8)
    }

    /// Decodes this window's section into `size` bytes, all its input used.
    pub(super) fn decode(&mut self, input: &[u8], size: usize) -> Result<Vec<u8>> {
        if size == 0 {
            return err("the patch is damaged (FGK: an empty section)");
        }
        let mut out = Vec::with_capacity(size);
        for (i, &byte) in input.iter().enumerate() {
            for bit in 0..8 {
                if self.decode_bit(byte & (1 << bit) != 0)? {
                    out.push(self.decode_data()?);
                    if out.len() == size {
                        if i + 1 != input.len() {
                            return err("the patch is damaged (FGK: unused input)");
                        }
                        return Ok(out);
                    }
                }
            }
        }
        err("the patch ends early (FGK: end of input)")
    }

    /// A section the encoder ran through its tree but left uncompressed: its tree changed all
    /// the same.
    pub(super) fn replay(&mut self, section: &[u8]) -> Result<()> {
        for &b in section {
            self.update_tree(usize::from(b))?;
        }
        // The tree's first symbol gives it a new root: decoding starts there.
        self.decode_ptr = self.root;
        Ok(())
    }
}
