//! The model behind `<sk-editor>`: a document of text (UTF-16 offsets), marks, atoms and a block
//! type per line, changed only by [`Transaction`]s, with an undo log and the editor state around
//! it. No layout and no gpui: the element in sk-elements draws it.

pub mod buffer;
pub mod doc;
pub mod editor;
pub mod history;
pub mod json;
pub mod transaction;
pub mod value;

pub use buffer::{utf16_len, SmallBuffer, TextBuffer};
pub use doc::{Document, Meta};
pub use editor::{Decoration, Editor, LineDecoration, Outcome};
pub use history::{History, StepKind};
pub use transaction::{
    map_pos, Applied, ChangeDesc, ChangeSpec, Insert, MarkRemoval, Selection, SetLines, TextOp,
    Transaction, TxError,
};
pub use value::{Atom, Block, EditorValue, LineSpec, Mark, ATOM_CHAR};
