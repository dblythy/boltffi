#[derive(Debug, Clone)]
pub struct DartClass {
    pub name: String,
    pub create_symbol: String,
    pub free_symbol: String,
    pub constructors: Vec<super::DartConstructor>,
    pub methods: Vec<super::DartFunction>,
    pub streams: Vec<super::DartStream>,
    /// `false` when this class owns a host callback its free/release may synchronously
    /// reenter — `isLeaf: true` on such a symbol aborts the VM (the same whole-class
    /// conservatism as `lower_method`'s heuristic, applied to the hardcoded free symbol).
    pub free_is_leaf: bool,
}
