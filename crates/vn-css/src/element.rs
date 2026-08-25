//! `parcel_selectors::Element` over an abstract DOM view (`MatchDom`), so the
//! matcher can walk vue-native's retained tree without vn-css depending on
//! vn-tree. The `ExtractImpl` projection recovers lightningcss's private
//! `Selectors` impl type through the public `Selector` alias — the one
//! non-obvious trick, proven by the research spike.

use lightningcss::selector::{PseudoClass, Selector as LcssSelector};
use parcel_selectors::attr::{AttrSelectorOperation, CaseSensitivity, NamespaceConstraint};
use parcel_selectors::context::MatchingContext;
use parcel_selectors::matching::ElementSelectorFlags;
use parcel_selectors::parser::SelectorImpl;
use parcel_selectors::OpaqueElement;

pub(crate) trait ExtractImpl {
    type I;
}
impl<'i, I: SelectorImpl<'i>> ExtractImpl for parcel_selectors::parser::Selector<'i, I> {
    type I = I;
}
/// `lightningcss::selector::Selectors`, recovered by projection.
pub(crate) type Impl = <LcssSelector<'static> as ExtractImpl>::I;

/// The minimal element view the matcher needs. Siblings are ELEMENT siblings
/// (text/anchor nodes skipped). `Copy` because matching walks freely.
pub trait MatchDom: Copy {
    fn parent(&self) -> Option<Self>;
    fn prev_sibling(&self) -> Option<Self>;
    fn next_sibling(&self) -> Option<Self>;
    fn has_tag(&self, name: &str) -> bool;
    fn tag_eq(&self, other: &Self) -> bool;
    fn has_class(&self, name: &str) -> bool;
    fn is_empty(&self) -> bool;
    fn is_root(&self) -> bool;
}

#[derive(Clone, Copy)]
pub(crate) struct El<D: MatchDom>(pub D);

impl<D: MatchDom> PartialEq for El<D> {
    fn eq(&self, other: &Self) -> bool {
        // Only used by the matcher for sibling/parent identity walks it
        // reached through us, where pointer-ish identity via tag_eq + position
        // is not required: parcel compares via `opaque()` in the paths we use.
        self.0.tag_eq(&other.0)
    }
}
impl<D: MatchDom> Eq for El<D> {}

impl<D: MatchDom> std::fmt::Debug for El<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "El")
    }
}

impl<'i, D: MatchDom> parcel_selectors::Element<'i> for El<D> {
    type Impl = Impl;

    fn opaque(&self) -> OpaqueElement {
        OpaqueElement::new(self)
    }

    fn parent_element(&self) -> Option<Self> {
        self.0.parent().map(El)
    }
    fn parent_node_is_shadow_root(&self) -> bool {
        false
    }
    fn containing_shadow_host(&self) -> Option<Self> {
        None
    }
    fn is_pseudo_element(&self) -> bool {
        false
    }
    fn prev_sibling_element(&self) -> Option<Self> {
        self.0.prev_sibling().map(El)
    }
    fn next_sibling_element(&self) -> Option<Self> {
        self.0.next_sibling().map(El)
    }
    fn is_html_element_in_html_document(&self) -> bool {
        false // class/id matching stays case sensitive
    }

    fn has_local_name(&self, local_name: &<Self::Impl as SelectorImpl<'i>>::BorrowedLocalName) -> bool {
        self.0.has_tag(local_name.0.as_ref())
    }
    fn has_namespace(&self, _ns: &<Self::Impl as SelectorImpl<'i>>::BorrowedNamespaceUrl) -> bool {
        true
    }
    fn is_same_type(&self, other: &Self) -> bool {
        self.0.tag_eq(&other.0)
    }

    fn attr_matches(
        &self,
        _ns: &NamespaceConstraint<&<Self::Impl as SelectorImpl<'i>>::NamespaceUrl>,
        _local_name: &<Self::Impl as SelectorImpl<'i>>::LocalName,
        _operation: &AttrSelectorOperation<&<Self::Impl as SelectorImpl<'i>>::AttrValue>,
    ) -> bool {
        false // attributes land with Vue scoped-CSS support
    }

    fn match_non_ts_pseudo_class<F>(
        &self,
        pc: &<Self::Impl as SelectorImpl<'i>>::NonTSPseudoClass,
        _context: &mut MatchingContext<'_, 'i, Self::Impl>,
        _flags_setter: &mut F,
    ) -> bool
    where
        F: FnMut(&Self, ElementSelectorFlags),
    {
        // The cascade computes "would this match WHEN in the state" — the
        // state buckets are classified separately and gpui picks them live.
        matches!(pc, PseudoClass::Hover | PseudoClass::Active)
    }

    fn match_pseudo_element(
        &self,
        _pe: &<Self::Impl as SelectorImpl<'i>>::PseudoElement,
        _context: &mut MatchingContext<'_, 'i, Self::Impl>,
    ) -> bool {
        false
    }

    fn is_link(&self) -> bool {
        false
    }
    fn is_html_slot_element(&self) -> bool {
        false
    }
    fn has_id(
        &self,
        _id: &<Self::Impl as SelectorImpl<'i>>::Identifier,
        _case_sensitivity: CaseSensitivity,
    ) -> bool {
        false // ids land with the attribute story
    }
    fn has_class(
        &self,
        name: &<Self::Impl as SelectorImpl<'i>>::Identifier,
        _case_sensitivity: CaseSensitivity,
    ) -> bool {
        self.0.has_class(name.0.as_ref())
    }
    fn imported_part(
        &self,
        _name: &<Self::Impl as SelectorImpl<'i>>::Identifier,
    ) -> Option<<Self::Impl as SelectorImpl<'i>>::Identifier> {
        None
    }
    fn is_part(&self, _name: &<Self::Impl as SelectorImpl<'i>>::Identifier) -> bool {
        false
    }
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    fn is_root(&self) -> bool {
        self.0.is_root()
    }
}
