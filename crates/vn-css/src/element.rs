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
    /// This element's tag name, for the rule index — the same string
    /// [`Self::has_tag`] compares against. (`has_tag` stays: the matcher asks
    /// the question that way, and a node kind with no tag can answer `false` to
    /// everything while still reporting `""` here.)
    fn tag_name(&self) -> &str;
    /// Calls `f` once per class on this element, for the rule index. Order is
    /// irrelevant — the cascade re-sorts by `(specificity, order)`.
    fn each_class(&self, f: &mut dyn FnMut(&str));
    /// Vue `<style scoped>` hash: does the node carry `data-v-...`?
    fn has_scope(&self, name: &str) -> bool;
    /// An attribute's value, or `None` when the node does not carry it. This is
    /// the DOM attribute channel — everything Vue assigns that is not `class`,
    /// `style` or an `on*` listener lands here, so it backs `[disabled]`,
    /// `[data-state="open"]`, `[aria-expanded]`, `#id`, `:disabled` and
    /// `:checked` alike. There is no separate "is the attribute present"
    /// question: a present attribute with no value is `Some("")`, as in HTML.
    fn attr(&self, name: &str) -> Option<&str>;
    fn is_empty(&self) -> bool;
    fn is_root(&self) -> bool;
}

/// The HTML rule for a boolean attribute (`disabled`, `checked`): present is
/// true whatever the value — except that Vue writes `:disabled="false"` through
/// as the string `"false"`, and `<input :disabled="false">` must NOT be
/// disabled. (In a browser Vue removes the attribute instead; our prop channel
/// keeps it and carries the value, so the value is what we read.)
fn boolean_attr<D: MatchDom>(node: &D, name: &str) -> bool {
    node.attr(name).is_some_and(|v| v != "false")
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
        local_name: &<Self::Impl as SelectorImpl<'i>>::LocalName,
        operation: &AttrSelectorOperation<&<Self::Impl as SelectorImpl<'i>>::AttrValue>,
    ) -> bool {
        // Vue scoped CSS: `[data-v-xxxx]` is a bare-presence test on the hot
        // path of every scoped rule, and scope hashes live in their own list
        // rather than the property map.
        let name: &str = local_name.0.as_ref();
        if name.starts_with("data-v-") {
            return matches!(operation, AttrSelectorOperation::Exists) && self.0.has_scope(name);
        }
        // Everything else comes off the property map. `eval_str` is
        // parcel's own operator implementation (`=`, `~=`, `|=`, `^=`, `$=`,
        // `*=`, plus the `i`/`s` case flags), so the semantics are the
        // browser's by construction.
        //
        // Namespaces are ignored: `@namespace` is not implemented, and the
        // audit rejects `[ns|attr]` at compile time rather than answering it
        // wrongly here.
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
            // Interaction states: the cascade computes "would this match WHEN
            // in the state" — the buckets are classified separately and gpui's
            // ladder picks them live, so here they are unconditionally true.
            PseudoClass::Hover
            | PseudoClass::Active
            | PseudoClass::Focus
            | PseudoClass::FocusVisible => true,
            // Property states: the element property IS the state, so these are
            // matched for real and their rules fold into the base bucket. A
            // property write already marks the tree dirty, so the next restyle
            // picks the change up — no ladder, no round trip.
            PseudoClass::Disabled => boolean_attr(&self.0, "disabled"),
            PseudoClass::Enabled => !boolean_attr(&self.0, "disabled"),
            PseudoClass::Checked => boolean_attr(&self.0, "checked"),
            // Everything else is rejected by the compile-time audit, so a rule
            // carrying it never reaches the matcher.
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
        // `id` is a plain DOM attribute on the property channel.
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
