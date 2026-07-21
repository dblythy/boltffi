use boltffi_ast::SourceName;
use boltffi_ffi_rules::naming as legacy_naming;

use crate::{NativeSymbol, SymbolId, SymbolName};

use super::LowerError;

pub const FFI_PREFIX: &str = "boltffi";

/// Which native-symbol naming scheme a lowering pass computes.
///
/// [`Experimental`](Self::Experimental) is the long-form, family- and
/// module-path-qualified scheme `boltffi_macros::experimental` compiles for
/// a crate that runs through the experimental macro expansion
/// (`BINDING_EXPANSION_BUILD_ENV`) — either because the crate's own build
/// script opts in (`boltffi_tests` is the only current example), OR because
/// `boltffi_cli`'s `BindingExpansion` sets that env var itself while
/// building the crate for real, which it does for the `apple`, `android`,
/// and `kotlin_multiplatform` pack commands (confirmed:
/// `boltffi_cli/src/build/expansion.rs`'s `env()`, and empirically — a real
/// `boltffi pack apple` xcframework's exported symbols are long-form,
/// matching this scheme, not `LegacyCompatible`). [`LegacyCompatible`](Self::LegacyCompatible)
/// mirrors `boltffi_ffi_rules::naming` exactly — the scheme the STABLE,
/// default macro path (`boltffi_macros::exports`/`lowering`) compiles for
/// any crate whose REAL SHIPPED ARTIFACT is a plain, unadorned `cargo build`
/// (`pack csharp` today — confirmed via `nm` against parse-core-rs's real
/// dylib: `_boltffi_parse_client_new`, `_boltffi_parse_client_become_user`,
/// ..., no family tag or module-path qualifier at all).
///
/// A metadata build (`boltffi_bindgen::metadata::BindingMetadataBuild`, read
/// by every native-target renderer) must request whichever style matches
/// how ITS CALLER actually builds the crate's real shipped artifact — there
/// is no crate-level default that's correct for everyone, because different
/// `boltffi_cli` pack commands build the real artifact differently. Getting
/// this wrong produces a P/Invoke-style entry point that compiles clean but
/// doesn't exist in the real library at runtime
/// (`EntryPointNotFoundException` class of bug —
/// `docs/tracks/boltffi-fork.md`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum NamingStyle {
    /// The in-progress macro rewrite's own long-form, collision-safe scheme.
    #[default]
    Experimental,
    /// Matches `boltffi_ffi_rules::naming` — the stable macro path's real,
    /// currently-shipping scheme.
    LegacyCompatible,
}

impl NamingStyle {
    /// Returns the stable metadata-build environment value for this style.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Experimental => "experimental",
            Self::LegacyCompatible => "legacy-compatible",
        }
    }

    /// Parses a metadata-build environment value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "experimental" => Some(Self::Experimental),
            "legacy-compatible" => Some(Self::LegacyCompatible),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
pub enum SymbolOwner<'source> {
    Record(&'source str),
    Enum(&'source str),
    Class(&'source str),
    Callback(&'source str),
}

impl<'source> SymbolOwner<'source> {
    pub const fn record(source_id: &'source str) -> Self {
        Self::Record(source_id)
    }

    pub const fn enumeration(source_id: &'source str) -> Self {
        Self::Enum(source_id)
    }

    pub const fn class(source_id: &'source str) -> Self {
        Self::Class(source_id)
    }

    pub const fn callback(source_id: &'source str) -> Self {
        Self::Callback(source_id)
    }

    pub fn method_symbol_name(self, member: &SourceName) -> String {
        format!(
            "{}_method_{}_{}_{}",
            FFI_PREFIX,
            self.family(),
            symbol_path(self.source_id()),
            source_member_name(member)
        )
    }

    pub fn initializer_symbol_name(self, initializer: &SourceName) -> String {
        format!(
            "{}_init_{}_{}_{}",
            FFI_PREFIX,
            self.family(),
            symbol_path(self.source_id()),
            source_member_name(initializer)
        )
    }

    fn family(self) -> &'static str {
        match self {
            Self::Record(_) => "record",
            Self::Enum(_) => "enum",
            Self::Class(_) => "class",
            Self::Callback(_) => "callback",
        }
    }

    fn source_id(self) -> &'source str {
        match self {
            Self::Record(source_id)
            | Self::Enum(source_id)
            | Self::Class(source_id)
            | Self::Callback(source_id) => source_id,
        }
    }

    /// The owner's own short name, ignoring its module path — what
    /// `boltffi_ffi_rules::naming`'s `class_name`/`callback_id` parameters
    /// expect (legacy has no family tag or module-path qualifier to carry).
    fn leaf_name(self) -> String {
        leaf(self.source_id())
    }

    /// Legacy-compatible: no family tag, no module-path qualifier — matches
    /// `boltffi_ffi_rules::naming::method_ffi_name`, the real symbol scheme
    /// the stable macro path compiles (verified against parse-core-rs's
    /// `_boltffi_parse_client_become_user`). Uses `member.spelling()` (the
    /// exact Rust source identifier), not `source_member_name`'s
    /// re-snake-cased canonical parts: the real macro
    /// (`boltffi_macros/src/exports/methods.rs`) passes the method's bare
    /// `to_string()`'d `syn::Ident` straight into `naming::method_ffi_name`
    /// with no case transform at all, so the two only coincide by
    /// convention (Rust method names already being snake_case), not by
    /// construction.
    pub fn legacy_method_symbol_name(self, member: &SourceName) -> String {
        legacy_naming::method_ffi_name(&self.leaf_name(), member.spelling()).into_string()
    }

    /// Legacy has no separate initializer lane: a class initializer compiles
    /// to the exact same `boltffi_<leaf>_<member>` shape as any other method
    /// (verified against parse-core-rs's real `_boltffi_parse_client_new`).
    pub fn legacy_initializer_symbol_name(self, initializer: &SourceName) -> String {
        self.legacy_method_symbol_name(initializer)
    }
}

/// The trailing `::`-segment of a fully qualified source id, snake-cased via
/// the same `boltffi_ffi_rules::naming::to_snake_case` the stable macro path
/// uses — the module path itself carries no weight in the legacy scheme.
fn leaf(source_id: &str) -> String {
    legacy_naming::to_snake_case(source_id.rsplit("::").next().unwrap_or(source_id))
}

pub struct SymbolAllocator {
    next: u32,
    style: NamingStyle,
}

impl SymbolAllocator {
    pub fn new() -> Self {
        Self {
            next: 0,
            style: NamingStyle::Experimental,
        }
    }

    /// Mints symbols matching the stable macro path's real compiled ABI —
    /// see [`NamingStyle::LegacyCompatible`].
    pub fn new_legacy_compatible() -> Self {
        Self {
            next: 0,
            style: NamingStyle::LegacyCompatible,
        }
    }

    pub fn mint(&mut self, name: String) -> Result<NativeSymbol, LowerError> {
        let id = self.next_id();
        let parsed = SymbolName::parse(name)?;
        Ok(NativeSymbol::new(id, parsed))
    }

    pub fn mint_function(&mut self, function_id: &str) -> Result<NativeSymbol, LowerError> {
        let name = match self.style {
            NamingStyle::Experimental => format!(
                "{}_function_{}",
                FFI_PREFIX,
                symbol_path(function_id)
            ),
            // matches parse-core-rs's real `_boltffi_fire_timer` etc: no
            // "function" tag, no module-path qualifier.
            NamingStyle::LegacyCompatible => {
                legacy_naming::function_ffi_name(&leaf(function_id)).into_string()
            }
        };
        self.mint(name)
    }

    /// Legacy has no constant-accessor lane at all (`boltffi_macros::exports`
    /// does not implement `#[export] const`) — a legacy-compiled crate can
    /// never reach this, so [`NamingStyle::LegacyCompatible`] mints the same
    /// experimental-style name as a harmless default rather than adding an
    /// unreachable branch.
    pub fn mint_constant_accessor(
        &mut self,
        constant_id: &str,
    ) -> Result<NativeSymbol, LowerError> {
        self.mint(format!("{}_const_{}", FFI_PREFIX, symbol_path(constant_id)))
    }

    pub fn mint_class_release(&mut self, class_id: &str) -> Result<NativeSymbol, LowerError> {
        let name = match self.style {
            NamingStyle::Experimental => format!(
                "{}_release_class_{}",
                FFI_PREFIX,
                symbol_path(class_id)
            ),
            // matches parse-core-rs's real `_boltffi_parse_client_free`.
            NamingStyle::LegacyCompatible => {
                legacy_naming::class_ffi_free(&leaf(class_id)).into_string()
            }
        };
        self.mint(name)
    }

    pub fn mint_method(
        &mut self,
        owner: SymbolOwner,
        member: &SourceName,
    ) -> Result<NativeSymbol, LowerError> {
        let name = match self.style {
            NamingStyle::Experimental => owner.method_symbol_name(member),
            NamingStyle::LegacyCompatible => owner.legacy_method_symbol_name(member),
        };
        self.mint(name)
    }

    pub fn mint_initializer(
        &mut self,
        owner: SymbolOwner,
        initializer: &SourceName,
    ) -> Result<NativeSymbol, LowerError> {
        let name = match self.style {
            NamingStyle::Experimental => owner.initializer_symbol_name(initializer),
            NamingStyle::LegacyCompatible => owner.legacy_initializer_symbol_name(initializer),
        };
        self.mint(name)
    }

    pub fn mint_callback_register(
        &mut self,
        callback_id: &str,
    ) -> Result<NativeSymbol, LowerError> {
        let name = match self.style {
            NamingStyle::Experimental => format!(
                "{}_register_callback_{}",
                FFI_PREFIX,
                symbol_path(callback_id)
            ),
            // matches parse-core-rs's real `_boltffi_register_clock_vtable`.
            NamingStyle::LegacyCompatible => {
                legacy_naming::callback_register_fn(&leaf(callback_id)).into_string()
            }
        };
        self.mint(name)
    }

    pub fn mint_callback_create_handle(
        &mut self,
        callback_id: &str,
    ) -> Result<NativeSymbol, LowerError> {
        let name = match self.style {
            NamingStyle::Experimental => format!(
                "{}_create_callback_{}",
                FFI_PREFIX,
                symbol_path(callback_id)
            ),
            // matches parse-core-rs's real `_boltffi_create_clock_handle`.
            NamingStyle::LegacyCompatible => {
                legacy_naming::callback_create_fn(&leaf(callback_id)).into_string()
            }
        };
        self.mint(name)
    }

    /// Not yet mapped for [`NamingStyle::LegacyCompatible`] — no confirmed
    /// legacy-compiled crate reaches an async callback completion lane
    /// today (unlike `mint_class_release`/`mint_method`/`mint_callback_register`,
    /// all confirmed against parse-core-rs's real dylib). Mints the
    /// experimental-style name unconditionally; widen this once a
    /// legacy-compatible caller needs it, verified against a real build the
    /// same way the mapped lanes were.
    pub fn mint_callback_complete(
        &mut self,
        callback_id: &str,
        slot: &CallbackSlot,
    ) -> Result<NativeSymbol, LowerError> {
        self.mint(format!(
            "{}_callback_{}_{}_complete",
            FFI_PREFIX,
            symbol_path(callback_id),
            slot.as_str()
        ))
    }

    pub fn mint_stream(
        &mut self,
        stream_id: &str,
        action: StreamLifecycle,
    ) -> Result<NativeSymbol, LowerError> {
        let name = match self.style {
            NamingStyle::Experimental => format!(
                "{}_stream_{}_{}",
                FFI_PREFIX,
                symbol_path(stream_id),
                action.suffix()
            ),
            // `boltffi_scan` always mints a stream's id as `<owner_class_full_path>::<stream_method>`
            // (`boltffi_scan/src/items/stream.rs`'s `StreamId::new(format!("{owner}::{stream_name}"))`
            // — `#[ffi_stream]` is only ever an impl-block method in the stable macro,
            // `boltffi_macros/src/exports/methods.rs`'s `generate_stream_exports`, which mints via
            // `naming::stream_ffi_*(class_name, stream_name)` — the exact scheme mirrored below.
            NamingStyle::LegacyCompatible => {
                let (class_portion, member) =
                    stream_id.rsplit_once("::").unwrap_or(("", stream_id));
                let class_leaf = leaf(class_portion);
                match action {
                    StreamLifecycle::Subscribe => {
                        legacy_naming::stream_ffi_subscribe(&class_leaf, member)
                    }
                    StreamLifecycle::PopBatch => {
                        legacy_naming::stream_ffi_pop_batch(&class_leaf, member)
                    }
                    StreamLifecycle::Wait => legacy_naming::stream_ffi_wait(&class_leaf, member),
                    StreamLifecycle::Poll => legacy_naming::stream_ffi_poll(&class_leaf, member),
                    StreamLifecycle::Unsubscribe => {
                        legacy_naming::stream_ffi_unsubscribe(&class_leaf, member)
                    }
                    StreamLifecycle::Free => legacy_naming::stream_ffi_free(&class_leaf, member),
                }
                .into_string()
            }
        };
        self.mint(name)
    }

    pub fn mint_async_lifecycle(
        &mut self,
        start_symbol_name: &str,
        action: AsyncLifecycle,
    ) -> Result<NativeSymbol, LowerError> {
        let start_without_prefix = start_symbol_name
            .strip_prefix(&format!("{FFI_PREFIX}_"))
            .unwrap_or(start_symbol_name);
        let name = match self.style {
            NamingStyle::Experimental => format!(
                "{}_async_{}_{}",
                FFI_PREFIX,
                start_without_prefix,
                action.suffix()
            ),
            // legacy has no "async" infix: `_boltffi_parse_client_become_user_poll`,
            // not `_boltffi_async_parse_client_become_user_poll` — the suffix lands
            // directly on the base method/initializer name.
            NamingStyle::LegacyCompatible => {
                format!("{}_{}_{}", FFI_PREFIX, start_without_prefix, action.suffix())
            }
        };
        self.mint(name)
    }

    pub const fn next_group_id(&self) -> u32 {
        self.next
    }

    fn next_id(&mut self) -> SymbolId {
        let id = SymbolId::from_raw(self.next);
        self.next += 1;
        id
    }
}

fn source_member_name(name: &SourceName) -> String {
    name.parts()
        .map(|part| to_snake_case(part.as_str()))
        .collect::<Vec<_>>()
        .join("_")
}

#[derive(Clone, Copy)]
pub enum CallbackLocalLifecycle {
    Handle,
    Free,
    Clone,
}

impl CallbackLocalLifecycle {
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Handle => "handle",
            Self::Free => "free",
            Self::Clone => "clone",
        }
    }

    pub fn function_name(self, callback_id: &str) -> String {
        format!(
            "__{}_local_{}_{}",
            FFI_PREFIX,
            symbol_path(callback_id),
            self.suffix()
        )
    }
}

pub fn callback_wasm_import_free_name(callback_id: &str) -> String {
    wasm_callback_import_name("lifecycle", &symbol_path(callback_id), "free")
}

pub fn callback_wasm_import_clone_name(callback_id: &str) -> String {
    wasm_callback_import_name("lifecycle", &symbol_path(callback_id), "clone")
}

#[derive(Clone, Copy)]
pub enum StreamLifecycle {
    Subscribe,
    PopBatch,
    Wait,
    Poll,
    Unsubscribe,
    Free,
}

impl StreamLifecycle {
    const fn suffix(self) -> &'static str {
        match self {
            Self::Subscribe => "subscribe",
            Self::PopBatch => "pop_batch",
            Self::Wait => "wait",
            Self::Poll => "poll",
            Self::Unsubscribe => "unsubscribe",
            Self::Free => "free",
        }
    }
}

#[derive(Clone, Copy)]
pub enum AsyncLifecycle {
    Poll,
    PollSync,
    Complete,
    Cancel,
    Free,
    Panic,
}

impl AsyncLifecycle {
    const fn suffix(self) -> &'static str {
        match self {
            Self::Poll => "poll",
            Self::PollSync => "poll_sync",
            Self::Complete => "complete",
            Self::Cancel => "cancel",
            Self::Free => "free",
            Self::Panic => "panic_message",
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CallbackSlot(String);

impl CallbackSlot {
    pub fn from_method_name(method_name: &str) -> Self {
        Self(to_snake_case(method_name))
    }

    pub fn from_source_name(name: &SourceName) -> Self {
        Self(source_member_name(name))
    }

    pub fn local_method_name(&self, callback_id: &str) -> String {
        format!(
            "__{}_local_{}_{}",
            FFI_PREFIX,
            symbol_path(callback_id),
            self.as_str()
        )
    }

    pub fn wasm_import_method_name(&self, callback_id: &str) -> String {
        wasm_callback_import_name("method", &symbol_path(callback_id), self.as_str())
    }

    pub fn wasm_import_start_name(&self, callback_id: &str) -> String {
        wasm_callback_import_name("async_start", &symbol_path(callback_id), self.as_str())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub const WASM_CALLBACK_IMPORT_MODULE: &str = "env";

pub const VTABLE_FREE_SLOT_NAME: &str = "free";

pub const VTABLE_CLONE_SLOT_NAME: &str = "clone";

fn symbol_path(source_id: &str) -> String {
    source_id
        .split("::")
        .filter(|segment| !segment.is_empty())
        .map(to_snake_case)
        .collect::<Vec<_>>()
        .join("_")
}

pub fn wasm_callback_import_name(lane: &str, owner: &str, action: &str) -> String {
    format!("__{}_callback_{}_{}_{}", FFI_PREFIX, lane, owner, action)
}

pub fn wasm_closure_export_name(group_id: u32, signature: &str, action: &str) -> String {
    format!(
        "{}_closure_{}_{}_{}",
        FFI_PREFIX, group_id, signature, action
    )
}

pub fn to_snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let initial = String::with_capacity(name.len() + chars.len() / 2);
    chars
        .iter()
        .enumerate()
        .fold(initial, |mut result, (index, &character)| {
            if character.is_uppercase() && index > 0 {
                let previous = chars[index - 1];
                let next = chars.get(index + 1).copied();
                let previous_is_word = previous.is_lowercase() || previous.is_ascii_digit();
                let acronym_word_break = previous.is_uppercase()
                    && next.is_some_and(|character| character.is_lowercase());
                if previous_is_word || acronym_word_break {
                    result.push('_');
                }
            }
            if character == '-' {
                result.push('_');
            } else {
                result.extend(character.to_lowercase());
            }
            result
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_lowercases_camel_words() {
        assert_eq!(to_snake_case("MyRecord"), "my_record");
        assert_eq!(to_snake_case("Point"), "point");
    }

    #[test]
    fn snake_case_breaks_acronyms_before_following_word() {
        assert_eq!(to_snake_case("HTTPHeader"), "http_header");
        assert_eq!(to_snake_case("XMLParser"), "xml_parser");
        assert_eq!(to_snake_case("MyHTTPClient"), "my_http_client");
    }

    #[test]
    fn snake_case_collapses_pure_acronyms() {
        assert_eq!(to_snake_case("HTTP"), "http");
        assert_eq!(to_snake_case("URL"), "url");
    }

    #[test]
    fn snake_case_passes_through_lowercase() {
        assert_eq!(to_snake_case("point"), "point");
        assert_eq!(to_snake_case("my_record"), "my_record");
    }

    #[test]
    fn snake_case_treats_digit_then_upper_as_word_break() {
        assert_eq!(to_snake_case("Point2D"), "point2_d");
        assert_eq!(to_snake_case("Vector3"), "vector3");
    }

    #[test]
    fn callback_local_handle_name_uses_local_callback_namespace() {
        assert_eq!(
            CallbackLocalLifecycle::Handle.function_name("demo::progress::ProgressListener"),
            "__boltffi_local_demo_progress_progress_listener_handle"
        );
    }

    #[test]
    fn member_symbol_name_uses_owner_and_member() {
        let member = SourceName::from_canonical(boltffi_ast::CanonicalName::new(vec![
            boltffi_ast::NamePart::new("translate"),
        ]));

        assert_eq!(
            SymbolOwner::record("demo::MyRecord").method_symbol_name(&member),
            "boltffi_method_record_demo_my_record_translate"
        );
    }

    #[test]
    fn source_member_name_snake_cases_each_source_part() {
        let name = SourceName::from_canonical(boltffi_ast::CanonicalName::new(vec![
            boltffi_ast::NamePart::new("from"),
            boltffi_ast::NamePart::new("HTTPRequest"),
        ]));

        assert_eq!(source_member_name(&name), "from_http_request");
    }

    #[test]
    fn initializer_symbol_name_uses_initializer_lane() {
        let initializer = SourceName::from_canonical(boltffi_ast::CanonicalName::new(vec![
            boltffi_ast::NamePart::new("new"),
        ]));

        assert_eq!(
            SymbolOwner::record("demo::Point").initializer_symbol_name(&initializer),
            "boltffi_init_record_demo_point_new"
        );
    }

    #[test]
    fn constant_accessor_symbol_name_uses_const_lane() {
        let mut allocator = SymbolAllocator::new();
        let symbol = allocator
            .mint_constant_accessor("demo::MAGIC")
            .expect("valid symbol");

        assert_eq!(symbol.name().as_str(), "boltffi_const_demo_magic");
    }

    #[test]
    fn class_release_symbol_name_uses_release_lane() {
        let mut allocator = SymbolAllocator::new();
        let symbol = allocator
            .mint_class_release("demo::Engine")
            .expect("valid symbol");

        assert_eq!(symbol.name().as_str(), "boltffi_release_class_demo_engine");
    }

    #[test]
    fn symbol_paths_include_source_namespaces() {
        let member = SourceName::from_canonical(boltffi_ast::CanonicalName::new(vec![
            boltffi_ast::NamePart::new("fetch"),
        ]));

        assert_eq!(
            SymbolOwner::class("demo::nested::HTTPClient").method_symbol_name(&member),
            "boltffi_method_class_demo_nested_http_client_fetch"
        );
    }

    #[test]
    fn allocator_mints_fresh_ids() {
        let mut allocator = SymbolAllocator::new();
        let first = allocator
            .mint("boltffi_demo_one".to_owned())
            .expect("valid name");
        let second = allocator
            .mint("boltffi_demo_two".to_owned())
            .expect("valid name");
        assert_ne!(first.id(), second.id());
        assert_eq!(first.id().raw(), 0);
        assert_eq!(second.id().raw(), 1);
    }

    #[test]
    fn wasm_callback_import_name_uses_shared_callback_lane() {
        assert_eq!(
            wasm_callback_import_name("method", "demo_listener", "on_event"),
            "__boltffi_callback_method_demo_listener_on_event"
        );
    }

    #[test]
    fn async_lifecycle_symbol_names_append_runtime_suffixes() {
        let mut allocator = SymbolAllocator::new();

        assert_eq!(
            allocator
                .mint_async_lifecycle("boltffi_function_demo_spin", AsyncLifecycle::Poll)
                .expect("valid symbol")
                .name()
                .as_str(),
            "boltffi_async_function_demo_spin_poll"
        );
        assert_eq!(
            allocator
                .mint_async_lifecycle("boltffi_function_demo_spin", AsyncLifecycle::PollSync)
                .expect("valid symbol")
                .name()
                .as_str(),
            "boltffi_async_function_demo_spin_poll_sync"
        );
        assert_eq!(
            allocator
                .mint_async_lifecycle("boltffi_function_demo_spin", AsyncLifecycle::Complete)
                .expect("valid symbol")
                .name()
                .as_str(),
            "boltffi_async_function_demo_spin_complete"
        );
    }

    #[test]
    fn wasm_async_callback_names_use_start_import_and_complete_export() {
        let slot = CallbackSlot::from_method_name("onEvent");
        let mut allocator = SymbolAllocator::new();

        assert_eq!(
            slot.wasm_import_start_name("demo::Listener"),
            "__boltffi_callback_async_start_demo_listener_on_event"
        );
        assert_eq!(
            allocator
                .mint_callback_complete("demo::Listener", &slot)
                .expect("valid symbol")
                .name()
                .as_str(),
            "boltffi_callback_demo_listener_on_event_complete"
        );
    }

    // `NamingStyle::LegacyCompatible` — every expected string below is the
    // REAL symbol observed via `nm` on parse-core-rs's dylib
    // (`_boltffi_parse_client_new`, `_boltffi_parse_client_become_user`,
    // `_boltffi_parse_client_free`, `_boltffi_fire_timer`,
    // `_boltffi_register_clock_vtable`, `_boltffi_create_clock_handle`), not
    // a value invented to match the implementation.

    #[test]
    fn legacy_initializer_symbol_name_has_no_family_tag_or_module_path() {
        let initializer = SourceName::from_canonical(boltffi_ast::CanonicalName::new(vec![
            boltffi_ast::NamePart::new("new"),
        ]));

        assert_eq!(
            SymbolOwner::class("parse_core::ffi::client::ParseClient")
                .legacy_initializer_symbol_name(&initializer),
            "boltffi_parse_client_new"
        );
    }

    #[test]
    fn legacy_method_symbol_name_has_no_family_tag_or_module_path() {
        let member = SourceName::from_canonical(boltffi_ast::CanonicalName::new(vec![
            boltffi_ast::NamePart::new("become_user"),
        ]));

        assert_eq!(
            SymbolOwner::class("parse_core::ffi::client::ParseClient")
                .legacy_method_symbol_name(&member),
            "boltffi_parse_client_become_user"
        );
    }

    #[test]
    fn legacy_method_symbol_name_uses_verbatim_spelling_not_the_re_snake_cased_canonical_name() {
        // A method name whose exact Rust spelling differs from what
        // re-snake-casing its canonical parts would produce — the real
        // stable macro (`boltffi_macros/src/exports/methods.rs`) passes
        // `method_name.to_string()` (the bare `syn::Ident`, verbatim)
        // straight into `naming::method_ffi_name` with no case transform at
        // all, so the legacy symbol must preserve `rawHTTPBody` exactly,
        // not re-derive `raw_http_body` from the canonical parts.
        let member = SourceName::new(
            "rawHTTPBody",
            boltffi_ast::CanonicalName::new(vec![
                boltffi_ast::NamePart::new("raw"),
                boltffi_ast::NamePart::new("HTTPBody"),
            ]),
        );

        assert_eq!(
            SymbolOwner::class("parse_core::ffi::client::ParseClient")
                .legacy_method_symbol_name(&member),
            "boltffi_parse_client_rawHTTPBody"
        );
    }

    #[test]
    fn legacy_method_symbol_name_applies_uniformly_to_record_owners_too() {
        let member = SourceName::from_canonical(boltffi_ast::CanonicalName::new(vec![
            boltffi_ast::NamePart::new("get_public_read_access"),
        ]));

        assert_eq!(
            SymbolOwner::record("parse_core::ffi::acl::Acl").legacy_method_symbol_name(&member),
            "boltffi_acl_get_public_read_access"
        );
    }

    #[test]
    fn legacy_style_allocator_mints_class_release_without_release_class_tag() {
        let mut allocator = SymbolAllocator::new_legacy_compatible();
        let symbol = allocator
            .mint_class_release("parse_core::ffi::client::ParseClient")
            .expect("valid symbol");

        assert_eq!(symbol.name().as_str(), "boltffi_parse_client_free");
    }

    #[test]
    fn legacy_style_allocator_mints_functions_without_function_tag_or_path() {
        let mut allocator = SymbolAllocator::new_legacy_compatible();
        let symbol = allocator
            .mint_function("parse_core::ffi::transport::fire_timer")
            .expect("valid symbol");

        assert_eq!(symbol.name().as_str(), "boltffi_fire_timer");
    }

    #[test]
    fn legacy_style_allocator_mints_callback_register_and_create_as_vtable_and_handle() {
        let mut allocator = SymbolAllocator::new_legacy_compatible();

        let register = allocator
            .mint_callback_register("parse_core::ffi::transport::Clock")
            .expect("valid symbol");
        let create = allocator
            .mint_callback_create_handle("parse_core::ffi::transport::Clock")
            .expect("valid symbol");

        assert_eq!(register.name().as_str(), "boltffi_register_clock_vtable");
        assert_eq!(create.name().as_str(), "boltffi_create_clock_handle");
    }

    #[test]
    fn legacy_style_allocator_mints_methods_via_the_shared_allocator_entry_point() {
        let member = SourceName::from_canonical(boltffi_ast::CanonicalName::new(vec![
            boltffi_ast::NamePart::new("become_user"),
        ]));
        let mut allocator = SymbolAllocator::new_legacy_compatible();

        let symbol = allocator
            .mint_method(
                SymbolOwner::class("parse_core::ffi::client::ParseClient"),
                &member,
            )
            .expect("valid symbol");

        assert_eq!(symbol.name().as_str(), "boltffi_parse_client_become_user");
    }

    #[test]
    fn legacy_style_async_lifecycle_suffixes_stay_correct_on_a_short_base_name() {
        let mut allocator = SymbolAllocator::new_legacy_compatible();
        let symbol = allocator
            .mint_async_lifecycle("boltffi_parse_client_become_user", AsyncLifecycle::Poll)
            .expect("valid symbol");

        assert_eq!(
            symbol.name().as_str(),
            "boltffi_parse_client_become_user_poll"
        );
    }

    #[test]
    fn legacy_style_stream_lifecycle_uses_owner_class_leaf_and_bare_stream_name() {
        let mut allocator = SymbolAllocator::new_legacy_compatible();

        let subscribe = allocator
            .mint_stream(
                "parse_core::ffi::watch::WatchHandle::changes",
                StreamLifecycle::Subscribe,
            )
            .expect("valid symbol");
        let pop_batch = allocator
            .mint_stream(
                "parse_core::ffi::watch::WatchHandle::changes",
                StreamLifecycle::PopBatch,
            )
            .expect("valid symbol");
        let free = allocator
            .mint_stream(
                "parse_core::ffi::watch::WatchHandle::changes",
                StreamLifecycle::Free,
            )
            .expect("valid symbol");

        assert_eq!(subscribe.name().as_str(), "boltffi_watch_handle_changes");
        assert_eq!(
            pop_batch.name().as_str(),
            "boltffi_watch_handle_changes_pop_batch"
        );
        assert_eq!(free.name().as_str(), "boltffi_watch_handle_changes_free");
    }
}
