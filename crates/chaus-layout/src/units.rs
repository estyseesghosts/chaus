//! Coordinate newtypes for text layout.
//!
//! Values from different coordinate spaces must not be interchangeable. A
//! scalar offset into a hard line, an expanded display column, a visual row
//! index and a UTF-8 byte offset in shaped text are distinct quantities even
//! though they all reduce to `usize` at an external API boundary.

macro_rules! layout_index {
    ($name:ident, $description:expr) => {
        #[doc = $description]
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub usize);

        impl $name {
            /// Construct the index from its underlying count.
            #[must_use]
            pub const fn new(value: usize) -> Self {
                Self(value)
            }

            /// Underlying count.
            #[must_use]
            pub const fn get(self) -> usize {
                self.0
            }
        }

        impl From<usize> for $name {
            fn from(value: usize) -> Self {
                Self(value)
            }
        }

        impl From<$name> for usize {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl PartialEq<usize> for $name {
            fn eq(&self, other: &usize) -> bool {
                self.0 == *other
            }
        }

        impl PartialEq<$name> for usize {
            fn eq(&self, other: &$name) -> bool {
                *self == other.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl PartialOrd<usize> for $name {
            fn partial_cmp(&self, other: &usize) -> Option<std::cmp::Ordering> {
                self.0.partial_cmp(other)
            }
        }

        impl PartialOrd<$name> for usize {
            fn partial_cmp(&self, other: &$name) -> Option<std::cmp::Ordering> {
                self.partial_cmp(&other.0)
            }
        }

        impl std::ops::Add<usize> for $name {
            type Output = usize;

            fn add(self, rhs: usize) -> Self::Output {
                self.0 + rhs
            }
        }

        impl std::ops::Sub<usize> for $name {
            type Output = usize;

            fn sub(self, rhs: usize) -> Self::Output {
                self.0 - rhs
            }
        }

        impl std::ops::AddAssign<usize> for $name {
            fn add_assign(&mut self, rhs: usize) {
                self.0 += rhs;
            }
        }

        impl std::iter::Sum<$name> for usize {
            fn sum<I: Iterator<Item = $name>>(iter: I) -> Self {
                iter.map(|value| value.0).sum()
            }
        }
    };
}

layout_index!(
    LineCharOffset,
    "Unicode scalar offset inside one hard line."
);
layout_index!(
    VisualColumn,
    "Display column inside a hard line, tabs expanded."
);
layout_index!(WrapRowIndex, "Visual row index inside one hard line.");
layout_index!(
    DisplayByteOffset,
    "UTF-8 byte offset inside expanded display text."
);
layout_index!(
    RowColumn,
    "Caret's preferred column relative to one visual row, not the hard line."
);
layout_index!(
    WrapWidthColumns,
    "Committed or requested wrap width in display columns."
);
