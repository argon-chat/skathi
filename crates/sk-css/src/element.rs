//! `parcel_selectors::Element` over an abstract DOM view (`MatchDom`), so the matcher can walk the
//! retained tree without sk-css depending on sk-tree. `ExtractImpl` recovers lightningcss's private
//! `Selectors` impl type through the public `Selector` alias.

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
pub(crate) type Impl = <LcssSelector<'static> as ExtractImpl>::I;

pub trait MatchDom: Copy {
    fn key(&self) -> u64;
    fn parent(&self) -> Option<Self>;
    fn prev_sibling(&self) -> Option<Self>;
    fn next_sibling(&self) -> Option<Self>;
    fn has_tag(&self, name: &str) -> bool;
    fn tag_eq(&self, other: &Self) -> bool;
    fn has_class(&self, name: &str) -> bool;
    fn tag_name(&self) -> &str;
    fn each_class(&self, f: &mut dyn FnMut(&str));
    fn has_scope(&self, name: &str) -> bool;
    fn attr(&self, name: &str) -> Option<&str>;
    fn is_empty(&self) -> bool;
    fn is_root(&self) -> bool;
    fn each_child(&self, _f: &mut dyn FnMut(Self)) {}
    /// The answer moves when a following sibling changes, though the asker is neither its ancestor
    /// nor after it; a dependency-driven restyle must hear of that here.
    fn note_following_sibling_anchor(&self) {}
    const LIVE_STATE: bool = false;
    fn interaction(&self, _state: Interaction) -> Option<bool> {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interaction {
    Hover,
    Active,
    Focus,
    FocusVisible,
    FocusWithin,
}

/// Present is true whatever the value, except that Vue writes `:disabled="false"` as the string
/// `"false"`, which must not disable.
fn boolean_attr<D: MatchDom>(node: &D, name: &str) -> bool {
    node.attr(name).is_some_and(|v| v != "false")
}

#[derive(Clone, Copy)]
pub(crate) struct El<D: MatchDom>(pub D);

impl<D: MatchDom> PartialEq for El<D> {
    fn eq(&self, other: &Self) -> bool {
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
        // `El` is a `Copy` wrapper, so `OpaqueElement::new(self)` would hand out the address of a
        // temporary: identity must come from the node key.
        let address = (self.0.key() as usize).wrapping_add(1).max(1);
        // SAFETY: see above — a dangling reference to a ZST at a non-null, trivially-aligned
        // address, immediately turned back into a pointer.
        OpaqueElement::new::<()>(unsafe { &*(address as *const ()) })
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
        false
    }

    fn has_local_name(
        &self,
        local_name: &<Self::Impl as SelectorImpl<'i>>::BorrowedLocalName,
    ) -> bool {
        let name = local_name.0.as_ref();
        self.0.has_tag(name) || (self.0.is_root() && matches!(name, "html" | "body"))
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
        local_name: &<Self::Impl as SelectorImpl<'i>>::LocalName,
        operation: &AttrSelectorOperation<&<Self::Impl as SelectorImpl<'i>>::AttrValue>,
    ) -> bool {
        // Vue scoped CSS: `[data-v-xxxx]` is a bare-presence test on the hot path of every scoped
        // rule, and scope hashes live in their own list rather than the property map.
        let name: &str = local_name.0.as_ref();
        if name.starts_with("data-v-") {
            return matches!(operation, AttrSelectorOperation::Exists) && self.0.has_scope(name);
        }
        match self.0.attr(name) {
            Some(value) => operation.eval_str(value),
            None => false,
        }
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
        match pc {
            PseudoClass::Hover => self.0.interaction(Interaction::Hover).unwrap_or(true),
            PseudoClass::Active => self.0.interaction(Interaction::Active).unwrap_or(true),
            PseudoClass::Focus => self.0.interaction(Interaction::Focus).unwrap_or(true),
            PseudoClass::FocusVisible => self
                .0
                .interaction(Interaction::FocusVisible)
                .unwrap_or(true),
            PseudoClass::FocusWithin => {
                self.0.interaction(Interaction::FocusWithin).unwrap_or(true)
            }
            PseudoClass::Disabled => boolean_attr(&self.0, "disabled"),
            PseudoClass::Enabled => !boolean_attr(&self.0, "disabled"),
            PseudoClass::Checked => boolean_attr(&self.0, "checked"),
            PseudoClass::Fullscreen(_) => self.0.attr(":fullscreen").is_some(),
            PseudoClass::Custom { name } => crate::has::matches(name.as_ref(), &self.0),
            _ => false,
        }
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
        id: &<Self::Impl as SelectorImpl<'i>>::Identifier,
        case_sensitivity: CaseSensitivity,
    ) -> bool {
        self.0
            .attr("id")
            .is_some_and(|v| case_sensitivity.eq(v.as_bytes(), id.0.as_ref().as_bytes()))
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
