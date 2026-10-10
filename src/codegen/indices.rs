#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub(super) struct FunctionIndex(u32);

impl FunctionIndex {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    pub const fn offset(self, offset: u32) -> Self {
        Self(self.0 + offset)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub(super) struct LocalIndex(u32);

impl LocalIndex {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub(super) struct TypeIndex(u32);

impl TypeIndex {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub(super) struct LabelDepth(u32);

impl LabelDepth {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }
}
