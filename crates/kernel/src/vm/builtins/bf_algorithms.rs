// Copyright (C) 2026 Ryan Daum <ryan.daum@gmail.com> This program is free
// software: you can redistribute it and/or modify it under the terms of the GNU
// Affero General Public License as published by the Free Software Foundation,
// version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU Affero General Public License for more
// details.
//
// You should have received a copy of the GNU Affero General Public License along
// with this program. If not, see <https://www.gnu.org/licenses/>.

//! Builtin functions for pure algorithms over supplied MOO values.
//!
//! These builtins operate on caller-provided values such as lists, maps,
//! symbols, and objects. They do not inspect world state or task permissions.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::LazyLock;

use ahash::HashMap;
use moor_compiler::offset_for_builtin;
use moor_var::{
    Associative, E_ARGS, E_INVARG, E_MAXREC, E_TYPE, Map, Symbol, Var, Variant, v_float, v_int,
    v_list, v_list_iter, v_map, v_sym,
};

use crate::vm::builtins::{
    BfCallState, BfErr, BfRet, BfRet::Ret, BuiltinFunction, BuiltinTickBudget, noise,
};

static VAR_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("var"));
static UNBOUND_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("unbound"));
static RAISE_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("raise"));
static LEAVE_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("leave"));
static MAX_DEPTH_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("max_depth"));
static MAX_BINDINGS_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("max_bindings"));
static MAX_SOLUTIONS_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("max_solutions"));
static MAX_STEPS_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("max_steps"));
static CASE_SENSITIVE_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("case_sensitive"));
static DEDUPE_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("dedupe"));
static NOT_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("not"));
static DIRECTIONS_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("directions"));
static CORNER_CUTTING_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("corner_cutting"));
static RETURN_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("return"));
static PATH_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("path"));
static COST_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("cost"));
static DETAIL_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("detail"));
static VISITED_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("visited"));
static FOUND_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("found"));
static MAX_NODES_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("max_nodes"));
static FROM_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("from"));
static TO_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("to"));
static EDGE_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("edge"));
static LIST_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("list"));
static ROWS_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("rows"));
static COMPLETE_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("complete"));
static CONSTRAINTS_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("constraints"));
static LT_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("lt"));
static LE_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("le"));
static GT_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("gt"));
static GE_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("ge"));
static EQ_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("eq"));
static NE_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("ne"));

const DEFAULT_TERM_MAX_DEPTH: usize = 64;
const DEFAULT_TERM_MAX_BINDINGS: usize = 256;
const DEFAULT_QUERY_MAX_DEPTH: usize = 32;
const DEFAULT_QUERY_MAX_SOLUTIONS: usize = 256;
const DEFAULT_QUERY_MAX_STEPS: usize = 10_000;
const INTERNAL_QUERY_VAR_PREFIX: &str = "__term_query:";

#[derive(Clone, Copy)]
struct TermOptions {
    unbound: UnboundMode,
    case_sensitive: bool,
    max_depth: usize,
    max_bindings: usize,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum UnboundMode {
    Raise,
    Leave,
}

impl Default for TermOptions {
    fn default() -> Self {
        Self {
            unbound: UnboundMode::Raise,
            case_sensitive: false,
            max_depth: DEFAULT_TERM_MAX_DEPTH,
            max_bindings: DEFAULT_TERM_MAX_BINDINGS,
        }
    }
}

impl TermOptions {
    fn equal(&self, left: &Var, right: &Var) -> bool {
        if self.case_sensitive {
            left.eq_case_sensitive(right)
        } else {
            left == right
        }
    }

    fn key(&self, value: Var) -> TermKey {
        TermKey {
            value,
            case_sensitive: self.case_sensitive,
        }
    }

    fn map_value(&self, map: &Map, key: &Var) -> Option<Var> {
        if self.case_sensitive {
            map.iter_ref()
                .find(|(candidate, _)| candidate.eq_case_sensitive(key))
                .map(|(_, value)| value.clone())
        } else {
            map.get(key).ok()
        }
    }
}

// Matching, result deduplication, and recursive-state detection must agree on
// equality. Case-folded string hashes are valid for exact equality too: case
// variants can share a hash but remain distinct keys. Float zero signs must
// share a hash because both comparison modes consider them equal.
#[derive(Clone)]
struct TermKey {
    value: Var,
    case_sensitive: bool,
}

impl PartialEq for TermKey {
    fn eq(&self, other: &Self) -> bool {
        self.case_sensitive == other.case_sensitive
            && if self.case_sensitive {
                self.value.eq_case_sensitive(&other.value)
            } else {
                self.value == other.value
            }
    }
}

impl Eq for TermKey {}

impl Hash for TermKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.case_sensitive.hash(state);
        hash_term_value(&self.value, state);
    }
}

fn hash_term_value<H: Hasher>(value: &Var, state: &mut H) {
    match value.variant() {
        Variant::Float(0.0) => 0_u64.hash(state),
        Variant::List(items) => {
            for item in items.iter_ref() {
                hash_term_value(item, state);
            }
        }
        Variant::Map(items) => {
            for (key, item) in items.iter_ref() {
                hash_term_value(key, state);
                hash_term_value(item, state);
            }
        }
        Variant::Flyweight(flyweight) => {
            flyweight.delegate().hash(state);
            for (name, item) in flyweight.slots_storage().iter() {
                name.hash(state);
                hash_term_value(item, state);
            }
            for item in flyweight.contents().iter_ref() {
                hash_term_value(item, state);
            }
        }
        _ => value.hash(state),
    }
}

type TermBindings = HashMap<Symbol, Var>;

#[derive(Clone, Copy)]
struct QueryOptions {
    term: TermOptions,
    max_solutions: usize,
    max_steps: usize,
    dedupe: bool,
    detail: bool,
    constraints: bool,
}

impl Default for QueryOptions {
    fn default() -> Self {
        Self {
            term: TermOptions {
                max_depth: DEFAULT_QUERY_MAX_DEPTH,
                ..TermOptions::default()
            },
            max_solutions: DEFAULT_QUERY_MAX_SOLUTIONS,
            max_steps: DEFAULT_QUERY_MAX_STEPS,
            dedupe: true,
            detail: false,
            constraints: false,
        }
    }
}

#[derive(Clone, Debug)]
struct QueryRule {
    head: Var,
    body: Vec<Var>,
}

#[derive(Clone, Copy)]
struct MatchOptions {
    term: TermOptions,
    max_solutions: usize,
    max_steps: usize,
    dedupe: bool,
    detail: bool,
}

impl Default for MatchOptions {
    fn default() -> Self {
        Self {
            term: TermOptions::default(),
            max_solutions: DEFAULT_QUERY_MAX_SOLUTIONS,
            max_steps: DEFAULT_QUERY_MAX_STEPS,
            dedupe: true,
            detail: false,
        }
    }
}

/// How many answers a search keeps: one past the limit when the caller asked whether the
/// search was complete.
fn solution_cap(max_solutions: usize, detail: bool) -> usize {
    if detail {
        max_solutions.saturating_add(1)
    } else {
        max_solutions
    }
}

/// The result of a bounded search: the answers, or with detail, the answers within the limit
/// and whether there were no more.
fn search_result(
    mut rows: Vec<Var>,
    max_solutions: usize,
    detail: bool,
    boolean: impl Fn(bool) -> Var,
) -> Var {
    if !detail {
        return v_list(&rows);
    }
    let complete = rows.len() <= max_solutions;
    rows.truncate(max_solutions);
    v_map(&[
        (v_sym(*ROWS_SYM), v_list(&rows)),
        (v_sym(*COMPLETE_SYM), boolean(complete)),
    ])
}

fn return_detail_option(value: &Var, builtin: &str) -> Result<bool, BfErr> {
    let mode = option_value_symbol(value, "return")?;
    if mode == *LIST_SYM {
        Ok(false)
    } else if mode == *DETAIL_SYM {
        Ok(true)
    } else {
        Err(BfErr::ErrValue(E_INVARG.msg(format!(
            "{builtin} return option must be 'list or 'detail"
        ))))
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum GridAstarReturn {
    Path,
    Cost,
    Detail,
}

#[derive(Clone, Copy)]
struct GridAstarOptions {
    directions: u8,
    corner_cutting: bool,
    return_mode: GridAstarReturn,
    max_nodes: Option<usize>,
}

impl Default for GridAstarOptions {
    fn default() -> Self {
        Self {
            directions: 8,
            corner_cutting: false,
            return_mode: GridAstarReturn::Path,
            max_nodes: None,
        }
    }
}

#[derive(Debug)]
struct GridAstarResult {
    path: Vec<Var>,
    cost: i64,
    visited: usize,
}

#[derive(Clone, Copy)]
struct GridOptions {
    directions: u8,
    corner_cutting: bool,
    max_nodes: Option<usize>,
}

impl Default for GridOptions {
    fn default() -> Self {
        Self {
            directions: 8,
            corner_cutting: false,
            max_nodes: None,
        }
    }
}

#[derive(Clone, Copy)]
struct GraphOptions {
    max_nodes: Option<usize>,
    max_steps: usize,
}

impl Default for GraphOptions {
    fn default() -> Self {
        Self {
            max_nodes: None,
            max_steps: DEFAULT_QUERY_MAX_STEPS,
        }
    }
}

struct Graph {
    nodes: Vec<Var>,
    adjacency: HashMap<Var, Vec<Var>>,
    indegree: HashMap<Var, usize>,
}

#[inline]
fn option_key_symbol(key: &Var) -> Result<Symbol, BfErr> {
    match key.variant() {
        Variant::Sym(sym) => Ok(sym),
        Variant::Str(s) => Ok(Symbol::mk(s.as_str())),
        _ => Err(BfErr::ErrValue(
            E_TYPE.msg("term builtin option keys must be strings or symbols"),
        )),
    }
}

#[inline]
fn option_value_symbol(value: &Var, option_name: &str) -> Result<Symbol, BfErr> {
    match value.variant() {
        Variant::Sym(sym) => Ok(sym),
        Variant::Str(s) => Ok(Symbol::mk(s.as_str())),
        _ => Err(BfErr::ErrValue(
            E_TYPE.msg(format!("{option_name} option must be a string or symbol")),
        )),
    }
}

fn positive_usize_option(value: &Var, option_name: &str) -> Result<usize, BfErr> {
    let Some(value) = value.as_integer() else {
        return Err(BfErr::ErrValue(
            E_TYPE.msg(format!("{option_name} option must be an integer")),
        ));
    };
    if value <= 0 {
        return Err(BfErr::ErrValue(
            E_INVARG.msg(format!("{option_name} option must be positive")),
        ));
    }
    Ok(value as usize)
}

fn parse_term_options(value: Option<&Var>) -> Result<TermOptions, BfErr> {
    let mut options = TermOptions::default();
    let Some(value) = value else {
        return Ok(options);
    };
    let Some(map) = value.as_map() else {
        return Err(BfErr::ErrValue(
            E_TYPE.msg("term builtin options must be a map"),
        ));
    };

    for (key, value) in map.iter_ref() {
        let key = option_key_symbol(key)?;
        if key == *UNBOUND_SYM {
            let mode = option_value_symbol(value, "unbound")?;
            if mode == *RAISE_SYM {
                options.unbound = UnboundMode::Raise;
            } else if mode == *LEAVE_SYM {
                options.unbound = UnboundMode::Leave;
            } else {
                return Err(BfErr::ErrValue(
                    E_INVARG.msg("unbound option must be 'raise or 'leave"),
                ));
            }
        } else if key == *MAX_DEPTH_SYM {
            options.max_depth = positive_usize_option(value, "max_depth")?;
        } else if key == *CASE_SENSITIVE_SYM {
            options.case_sensitive = value.is_true();
        } else if key == *MAX_BINDINGS_SYM {
            options.max_bindings = positive_usize_option(value, "max_bindings")?;
        } else {
            return Err(BfErr::ErrValue(
                E_INVARG.msg(format!("unknown term builtin option: {key}")),
            ));
        }
    }

    Ok(options)
}

fn parse_query_options(value: Option<&Var>) -> Result<QueryOptions, BfErr> {
    let mut options = QueryOptions::default();
    let Some(value) = value else {
        return Ok(options);
    };
    let Some(map) = value.as_map() else {
        return Err(BfErr::ErrValue(
            E_TYPE.msg("term_query() options must be a map"),
        ));
    };

    for (key, value) in map.iter_ref() {
        let key = option_key_symbol(key)?;
        if key == *MAX_DEPTH_SYM {
            options.term.max_depth = positive_usize_option(value, "max_depth")?;
        } else if key == *MAX_BINDINGS_SYM {
            options.term.max_bindings = positive_usize_option(value, "max_bindings")?;
        } else if key == *MAX_SOLUTIONS_SYM {
            options.max_solutions = positive_usize_option(value, "max_solutions")?;
        } else if key == *MAX_STEPS_SYM {
            options.max_steps = positive_usize_option(value, "max_steps")?;
        } else if key == *CASE_SENSITIVE_SYM {
            options.term.case_sensitive = value.is_true();
        } else if key == *DEDUPE_SYM {
            options.dedupe = value.is_true();
        } else if key == *RETURN_SYM {
            options.detail = return_detail_option(value, "term_query()")?;
        } else if key == *CONSTRAINTS_SYM {
            options.constraints = value.is_true();
        } else {
            return Err(BfErr::ErrValue(
                E_INVARG.msg(format!("unknown term_query() option: {key}")),
            ));
        }
    }

    Ok(options)
}

fn parse_match_options(value: Option<&Var>) -> Result<MatchOptions, BfErr> {
    let mut options = MatchOptions::default();
    let Some(value) = value else {
        return Ok(options);
    };
    let Some(map) = value.as_map() else {
        return Err(BfErr::ErrValue(
            E_TYPE.msg("term_match() options must be a map"),
        ));
    };

    for (key, value) in map.iter_ref() {
        let key = option_key_symbol(key)?;
        if key == *MAX_DEPTH_SYM {
            options.term.max_depth = positive_usize_option(value, "max_depth")?;
        } else if key == *MAX_BINDINGS_SYM {
            options.term.max_bindings = positive_usize_option(value, "max_bindings")?;
        } else if key == *MAX_SOLUTIONS_SYM {
            options.max_solutions = positive_usize_option(value, "max_solutions")?;
        } else if key == *MAX_STEPS_SYM {
            options.max_steps = positive_usize_option(value, "max_steps")?;
        } else if key == *CASE_SENSITIVE_SYM {
            options.term.case_sensitive = value.is_true();
        } else if key == *DEDUPE_SYM {
            options.dedupe = value.is_true();
        } else if key == *RETURN_SYM {
            options.detail = return_detail_option(value, "term_match()")?;
        } else {
            return Err(BfErr::ErrValue(
                E_INVARG.msg(format!("unknown term_match() option: {key}")),
            ));
        }
    }

    Ok(options)
}

fn parse_grid_astar_options(value: Option<&Var>) -> Result<GridAstarOptions, BfErr> {
    let mut options = GridAstarOptions::default();
    let Some(value) = value else {
        return Ok(options);
    };
    let Some(map) = value.as_map() else {
        return Err(BfErr::ErrValue(
            E_TYPE.msg("grid_astar() options must be a map"),
        ));
    };

    for (key, value) in map.iter_ref() {
        let key = option_key_symbol(key)?;
        if key == *DIRECTIONS_SYM {
            let Some(directions) = value.as_integer() else {
                return Err(BfErr::ErrValue(
                    E_TYPE.msg("directions option must be an integer"),
                ));
            };
            if directions != 4 && directions != 8 {
                return Err(BfErr::ErrValue(
                    E_INVARG.msg("directions option must be 4 or 8"),
                ));
            }
            options.directions = directions as u8;
        } else if key == *CORNER_CUTTING_SYM {
            options.corner_cutting = value.is_true();
        } else if key == *RETURN_SYM {
            let mode = option_value_symbol(value, "return")?;
            if mode == *PATH_SYM {
                options.return_mode = GridAstarReturn::Path;
            } else if mode == *COST_SYM {
                options.return_mode = GridAstarReturn::Cost;
            } else if mode == *DETAIL_SYM {
                options.return_mode = GridAstarReturn::Detail;
            } else {
                return Err(BfErr::ErrValue(
                    E_INVARG.msg("return option must be 'path, 'cost, or 'detail"),
                ));
            }
        } else if key == *MAX_NODES_SYM {
            options.max_nodes = Some(positive_usize_option(value, "max_nodes")?);
        } else {
            return Err(BfErr::ErrValue(
                E_INVARG.msg(format!("unknown grid_astar() option: {key}")),
            ));
        }
    }

    Ok(options)
}

fn parse_grid_options(value: Option<&Var>) -> Result<GridOptions, BfErr> {
    let mut options = GridOptions::default();
    let Some(value) = value else {
        return Ok(options);
    };
    let Some(map) = value.as_map() else {
        return Err(BfErr::ErrValue(
            E_TYPE.msg("grid helper options must be a map"),
        ));
    };

    for (key, value) in map.iter_ref() {
        let key = option_key_symbol(key)?;
        if key == *DIRECTIONS_SYM {
            let Some(directions) = value.as_integer() else {
                return Err(BfErr::ErrValue(
                    E_TYPE.msg("directions option must be an integer"),
                ));
            };
            if directions != 4 && directions != 8 {
                return Err(BfErr::ErrValue(
                    E_INVARG.msg("directions option must be 4 or 8"),
                ));
            }
            options.directions = directions as u8;
        } else if key == *CORNER_CUTTING_SYM {
            options.corner_cutting = value.is_true();
        } else if key == *MAX_NODES_SYM {
            options.max_nodes = Some(positive_usize_option(value, "max_nodes")?);
        } else {
            return Err(BfErr::ErrValue(
                E_INVARG.msg(format!("unknown grid helper option: {key}")),
            ));
        }
    }

    Ok(options)
}

fn parse_graph_options(value: Option<&Var>) -> Result<GraphOptions, BfErr> {
    let mut options = GraphOptions::default();
    let Some(value) = value else {
        return Ok(options);
    };
    let Some(map) = value.as_map() else {
        return Err(BfErr::ErrValue(
            E_TYPE.msg("graph builtin options must be a map"),
        ));
    };

    for (key, value) in map.iter_ref() {
        let key = option_key_symbol(key)?;
        if key == *MAX_NODES_SYM {
            options.max_nodes = Some(positive_usize_option(value, "max_nodes")?);
        } else if key == *MAX_STEPS_SYM {
            options.max_steps = positive_usize_option(value, "max_steps")?;
        } else {
            return Err(BfErr::ErrValue(
                E_INVARG.msg(format!("unknown graph builtin option: {key}")),
            ));
        }
    }

    Ok(options)
}

fn positive_dimension(value: &Var, name: &str) -> Result<usize, BfErr> {
    match value.variant() {
        Variant::Int(i) if i > 0 => Ok(i as usize),
        _ => Err(BfErr::ErrValue(
            E_INVARG.msg(format!("{name} must be a positive integer")),
        )),
    }
}

fn integer_coordinate(value: &Var, name: &str) -> Result<i32, BfErr> {
    match value.variant() {
        Variant::Int(i) => Ok(i as i32),
        _ => Err(BfErr::ErrValue(
            E_TYPE.msg(format!("{name} must be an integer")),
        )),
    }
}

fn check_grid_position(
    width: usize,
    height: usize,
    x: i32,
    y: i32,
    name: &str,
) -> Result<(), BfErr> {
    if x < 0 || x >= width as i32 || y < 0 || y >= height as i32 {
        return Err(BfErr::ErrValue(
            E_INVARG.msg(format!("{name} position out of bounds")),
        ));
    }
    Ok(())
}

fn parse_passable_grid(
    width: usize,
    height: usize,
    tile_map: &Var,
    solid_tiles: &Var,
) -> Result<Vec<bool>, BfErr> {
    let tile_map_list = tile_map
        .as_list()
        .ok_or_else(|| BfErr::ErrValue(E_TYPE.msg("tile_map must be a list")))?;
    let solid_tiles_list = solid_tiles
        .as_list()
        .ok_or_else(|| BfErr::ErrValue(E_TYPE.msg("solid_tiles must be a list")))?;

    let grid_size = width * height;
    let mut solid_ids = std::collections::HashSet::new();
    for item in solid_tiles_list.iter() {
        if let Variant::Int(id) = item.variant() {
            solid_ids.insert(id);
        }
    }

    let mut passable = vec![true; grid_size];
    for (i, item) in tile_map_list.iter().enumerate() {
        if i >= grid_size {
            break;
        }
        if let Variant::Int(tile_id) = item.variant()
            && solid_ids.contains(&tile_id)
        {
            passable[i] = false;
        }
    }
    Ok(passable)
}

fn load_bindings(value: Option<&Var>, max_bindings: usize) -> Result<TermBindings, BfErr> {
    let mut bindings = TermBindings::default();
    let Some(value) = value else {
        return Ok(bindings);
    };
    let Some(map) = value.as_map() else {
        return Err(BfErr::ErrValue(E_TYPE.msg("term bindings must be a map")));
    };
    if map.len() > max_bindings {
        return Err(BfErr::ErrValue(
            E_INVARG.msg("term bindings exceed max_bindings"),
        ));
    }

    for (key, value) in map.iter_ref() {
        let Variant::Sym(name) = key.variant() else {
            return Err(BfErr::ErrValue(
                E_INVARG.msg("term binding keys must be symbols"),
            ));
        };
        bindings.insert(name, value.clone());
    }
    Ok(bindings)
}

/// One binding map, or a list of them to match each against every value.
fn load_binding_sets(value: Option<&Var>, max_bindings: usize) -> Result<Vec<TermBindings>, BfErr> {
    let Some(list) = value.and_then(|value| value.as_list()) else {
        return Ok(vec![load_bindings(value, max_bindings)?]);
    };
    list.iter_ref()
        .map(|bindings| load_bindings(Some(bindings), max_bindings))
        .collect()
}

fn bindings_to_var(bindings: TermBindings) -> Var {
    let pairs = bindings
        .into_iter()
        .map(|(name, value)| (v_sym(name), value))
        .collect::<Vec<_>>();
    v_map(&pairs)
}

fn variable_marker(term: &Var) -> Result<Option<Symbol>, BfErr> {
    let Variant::List(list) = term.variant() else {
        return Ok(None);
    };
    if list.is_empty() {
        return Ok(None);
    }
    let Variant::Sym(marker) = list[0].variant() else {
        return Ok(None);
    };
    if marker != *VAR_SYM {
        return Ok(None);
    }
    if list.len() != 2 {
        return Err(BfErr::ErrValue(
            E_INVARG.msg("variable marker must have shape {'var, symbol}"),
        ));
    }
    let Variant::Sym(name) = list[1].variant() else {
        return Err(BfErr::ErrValue(
            E_INVARG.msg("variable marker name must be a symbol"),
        ));
    };
    Ok(Some(name))
}

#[inline]
fn check_depth(depth: usize, options: &TermOptions) -> Result<(), BfErr> {
    if depth > options.max_depth {
        return Err(BfErr::ErrValue(
            E_MAXREC.msg("term recursion exceeded max_depth"),
        ));
    }
    Ok(())
}

fn bind_term_unify_variable(
    name: Symbol,
    datum: &Var,
    bindings: &mut TermBindings,
    options: &TermOptions,
) -> Result<bool, BfErr> {
    if let Some(bound) = bindings.get(&name) {
        return Ok(options.equal(bound, datum));
    }
    if bindings.len() >= options.max_bindings {
        return Err(BfErr::ErrValue(
            E_INVARG.msg("term_unify() exceeded max_bindings"),
        ));
    }
    bindings.insert(name, datum.clone());
    Ok(true)
}

fn term_unify_inner(
    pattern: &Var,
    datum: &Var,
    bindings: &mut TermBindings,
    options: &TermOptions,
    depth: usize,
) -> Result<bool, BfErr> {
    check_depth(depth, options)?;

    if let Some(name) = variable_marker(pattern)? {
        return bind_term_unify_variable(name, datum, bindings, options);
    }

    match (pattern.variant(), datum.variant()) {
        (Variant::List(pattern_list), Variant::List(datum_list)) => {
            if pattern_list.len() != datum_list.len() {
                return Ok(false);
            }
            for (pattern_item, datum_item) in pattern_list.iter_ref().zip(datum_list.iter_ref()) {
                if !term_unify_inner(pattern_item, datum_item, bindings, options, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Variant::Map(pattern_map), Variant::Map(datum_map)) => {
            if pattern_map.len() != datum_map.len() {
                return Ok(false);
            }
            for (key, pattern_value) in pattern_map.iter_ref() {
                let Some(datum_value) = options.map_value(datum_map, key) else {
                    return Ok(false);
                };
                if !term_unify_inner(pattern_value, &datum_value, bindings, options, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(options.equal(pattern, datum)),
    }
}

/// Usage: `map|bool term_unify(any pattern, any value [, map bindings [, map options]])`
///
/// Matches a pattern containing `{'var, symbol}` markers against exactly one value.
fn bf_term_unify(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.len() < 2 || bf_args.args.len() > 4 {
        return Err(BfErr::ErrValue(
            E_ARGS.msg("term_unify() takes 2 to 4 arguments"),
        ));
    }

    let options = parse_term_options(bf_args.args.iter_ref().nth(3))?;
    let mut bindings = load_bindings(bf_args.args.iter_ref().nth(2), options.max_bindings)?;
    let matched = term_unify_inner(
        &bf_args.args[0],
        &bf_args.args[1],
        &mut bindings,
        &options,
        0,
    )?;

    if matched {
        Ok(Ret(bindings_to_var(bindings)))
    } else {
        Ok(Ret(bf_args.v_bool(false)))
    }
}

fn term_match_values(
    pattern: &Var,
    values: &[Var],
    binding_sets: &[TermBindings],
    options: &MatchOptions,
    mut tick_budget: Option<BuiltinTickBudget<'_>>,
) -> Result<Vec<Var>, BfErr> {
    let cap = solution_cap(options.max_solutions, options.detail);
    let mut steps = 0usize;
    let mut results = Vec::new();
    let mut seen = HashSet::new();

    'sets: for initial_bindings in binding_sets {
        for value in values {
            if results.len() >= cap {
                break 'sets;
            }
            if let Some(tick_budget) = tick_budget.as_mut() {
                tick_budget.consume_ticks(1)?;
            }
            steps += 1;
            if steps > options.max_steps {
                return Err(BfErr::ErrValue(
                    E_MAXREC.msg("term_match() exceeded max_steps"),
                ));
            }

            let mut bindings = initial_bindings.clone();
            if !term_unify_inner(pattern, value, &mut bindings, &options.term, 0)? {
                continue;
            }
            let result = bindings_to_var(bindings);
            if options.dedupe && !seen.insert(options.term.key(result.clone())) {
                continue;
            }
            results.push(result);
        }
    }

    Ok(results)
}

/// Usage: `list|map term_match(any pattern, list values [, map|list bindings [, map options]])`
///
/// Applies one pattern to each candidate value and returns successful binding maps. Given a
/// list of binding maps, matches each against every value, in order.
fn bf_term_match(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.len() < 2 || bf_args.args.len() > 4 {
        return Err(BfErr::ErrValue(
            E_ARGS.msg("term_match() takes 2 to 4 arguments"),
        ));
    }

    let Some(values) = bf_args.args[1].as_list() else {
        return Err(BfErr::ErrValue(
            E_TYPE.msg("term_match() values must be a list"),
        ));
    };
    let options = parse_match_options(bf_args.args.iter_ref().nth(3))?;
    let binding_sets =
        load_binding_sets(bf_args.args.iter_ref().nth(2), options.term.max_bindings)?;
    let values = values.iter_ref().cloned().collect::<Vec<_>>();
    let results = term_match_values(
        &bf_args.args[0],
        &values,
        &binding_sets,
        &options,
        Some(bf_args.tick_budget()),
    )?;
    Ok(Ret(search_result(
        results,
        options.max_solutions,
        options.detail,
        |complete| bf_args.v_bool(complete),
    )))
}

fn term_substitute_inner(
    term: &Var,
    bindings: &TermBindings,
    options: &TermOptions,
    depth: usize,
) -> Result<(Var, bool), BfErr> {
    check_depth(depth, options)?;

    if let Some(name) = variable_marker(term)? {
        if let Some(value) = bindings.get(&name) {
            return Ok((value.clone(), true));
        }
        return match options.unbound {
            UnboundMode::Raise => Err(BfErr::ErrValue(
                E_INVARG.msg(format!("unbound variable in term_substitute(): {name}")),
            )),
            UnboundMode::Leave => Ok((term.clone(), false)),
        };
    }

    match term.variant() {
        Variant::List(list) => {
            let mut changed_items = None::<Vec<Var>>;
            for (index, item) in list.iter_ref().enumerate() {
                let (new_item, changed) =
                    term_substitute_inner(item, bindings, options, depth + 1)?;
                if let Some(items) = changed_items.as_mut() {
                    items.push(new_item);
                } else if changed {
                    let mut items = Vec::with_capacity(list.len());
                    items.extend(list.iter_ref().take(index).cloned());
                    items.push(new_item);
                    changed_items = Some(items);
                }
            }
            if let Some(items) = changed_items {
                Ok((v_list(&items), true))
            } else {
                Ok((term.clone(), false))
            }
        }
        Variant::Map(map) => {
            let mut changed_pairs = None::<Vec<(Var, Var)>>;
            for (index, (key, value)) in map.iter_ref().enumerate() {
                let (new_value, value_changed) =
                    term_substitute_inner(value, bindings, options, depth + 1)?;
                if let Some(pairs) = changed_pairs.as_mut() {
                    pairs.push((key.clone(), new_value));
                } else if value_changed {
                    let mut pairs = Vec::with_capacity(map.len());
                    pairs.extend(
                        map.iter_ref()
                            .take(index)
                            .map(|(prev_key, prev_value)| (prev_key.clone(), prev_value.clone())),
                    );
                    pairs.push((key.clone(), new_value));
                    changed_pairs = Some(pairs);
                }
            }
            if let Some(pairs) = changed_pairs {
                Ok((v_map(&pairs), true))
            } else {
                Ok((term.clone(), false))
            }
        }
        _ => Ok((term.clone(), false)),
    }
}

/// Usage: `any term_substitute(any template, map bindings [, map options])`
///
/// Recursively replaces `{'var, symbol}` markers in a template using a bindings map.
fn bf_term_substitute(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.len() < 2 || bf_args.args.len() > 3 {
        return Err(BfErr::ErrValue(
            E_ARGS.msg("term_substitute() takes 2 or 3 arguments"),
        ));
    }

    let options = parse_term_options(bf_args.args.iter_ref().nth(2))?;
    let bindings = load_bindings(bf_args.args.iter_ref().nth(1), options.max_bindings)?;
    let (result, _) = term_substitute_inner(&bf_args.args[0], &bindings, &options, 0)?;
    Ok(Ret(result))
}

/// Usage: `list term_variables(any term [, map options])`
///
/// Returns the unique `{'var, symbol}` names in traversal order.
fn bf_term_variables(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.is_empty() || bf_args.args.len() > 2 {
        return Err(BfErr::ErrValue(
            E_ARGS.msg("term_variables() takes 1 or 2 arguments"),
        ));
    }

    let options = parse_term_options(bf_args.args.iter_ref().nth(1))?;
    let mut variables = Vec::new();
    collect_variables_inner(&bf_args.args[0], &mut variables, &options, 0)?;
    Ok(Ret(v_list_iter(variables.into_iter().map(v_sym))))
}

/// Usage: `bool term_ground(any term [, map options])`
///
/// Returns true when a term contains no `{'var, symbol}` markers.
fn bf_term_ground(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.is_empty() || bf_args.args.len() > 2 {
        return Err(BfErr::ErrValue(
            E_ARGS.msg("term_ground() takes 1 or 2 arguments"),
        ));
    }

    let options = parse_term_options(bf_args.args.iter_ref().nth(1))?;
    let ground = term_is_ground_inner(&bf_args.args[0], &options, 0)?;
    Ok(Ret(bf_args.v_bool(ground)))
}

/// Usage: `any term_normalize(any term [, map bindings [, map options]])`
///
/// Resolves supplied bindings and canonicalizes remaining variables by encounter order.
fn bf_term_normalize(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.is_empty() || bf_args.args.len() > 3 {
        return Err(BfErr::ErrValue(
            E_ARGS.msg("term_normalize() takes 1 to 3 arguments"),
        ));
    }

    let options = parse_term_options(bf_args.args.iter_ref().nth(2))?;
    let bindings = load_bindings(bf_args.args.iter_ref().nth(1), options.max_bindings)?;
    let mut names = TermBindings::default();
    let normalized = term_normalize_inner(&bf_args.args[0], &bindings, &mut names, &options, 0)?;
    Ok(Ret(normalized))
}

#[inline]
fn variable_term(name: Symbol) -> Var {
    v_list(&[v_sym(*VAR_SYM), v_sym(name)])
}

#[inline]
fn is_internal_query_var(name: Symbol) -> bool {
    name.as_str().starts_with(INTERNAL_QUERY_VAR_PREFIX)
}

fn collect_variables_inner(
    term: &Var,
    variables: &mut Vec<Symbol>,
    options: &TermOptions,
    depth: usize,
) -> Result<(), BfErr> {
    check_depth(depth, options)?;

    if let Some(name) = variable_marker(term)? {
        if !variables.contains(&name) {
            variables.push(name);
        }
        return Ok(());
    }

    match term.variant() {
        Variant::List(list) => {
            for item in list.iter_ref() {
                collect_variables_inner(item, variables, options, depth + 1)?;
            }
        }
        Variant::Map(map) => {
            for (_, value) in map.iter_ref() {
                collect_variables_inner(value, variables, options, depth + 1)?;
            }
        }
        _ => {}
    }

    Ok(())
}

fn collect_variables(term: &Var, variables: &mut Vec<Symbol>) -> Result<(), BfErr> {
    collect_variables_inner(term, variables, &TermOptions::default(), 0)
}

fn term_is_ground_inner(term: &Var, options: &TermOptions, depth: usize) -> Result<bool, BfErr> {
    check_depth(depth, options)?;
    if variable_marker(term)?.is_some() {
        return Ok(false);
    }

    match term.variant() {
        Variant::List(list) => {
            for item in list.iter_ref() {
                if !term_is_ground_inner(item, options, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Variant::Map(map) => {
            for (_, value) in map.iter_ref() {
                if !term_is_ground_inner(value, options, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(true),
    }
}

fn body_term_is_negative(term: &Var) -> bool {
    let Variant::List(list) = term.variant() else {
        return false;
    };
    if list.is_empty() {
        return false;
    }
    matches!(list[0].variant(), Variant::Sym(name) if name == *NOT_SYM)
}

fn parse_query_rules(rules: &Var) -> Result<Vec<QueryRule>, BfErr> {
    let Some(rules) = rules.as_list() else {
        return Err(BfErr::ErrValue(
            E_TYPE.msg("term_query() rules must be a list"),
        ));
    };

    let mut parsed = Vec::with_capacity(rules.len());
    for rule in rules.iter_ref() {
        let Variant::List(pair) = rule.variant() else {
            return Err(BfErr::ErrValue(
                E_INVARG.msg("term_query() rules must have shape {head, body}"),
            ));
        };
        if pair.len() != 2 {
            return Err(BfErr::ErrValue(
                E_INVARG.msg("term_query() rules must have shape {head, body}"),
            ));
        }

        let head = pair[0].clone();
        let Some(body_list) = pair[1].as_list() else {
            return Err(BfErr::ErrValue(
                E_INVARG.msg("term_query() rule body must be a list of positive terms"),
            ));
        };

        let mut body = Vec::with_capacity(body_list.len());
        for term in body_list.iter_ref() {
            if body_term_is_negative(term) {
                return Err(BfErr::ErrValue(
                    E_INVARG.msg("term_query() v1 does not support negation"),
                ));
            }
            body.push(term.clone());
        }

        let mut head_vars = Vec::new();
        let mut body_vars = Vec::new();
        collect_variables(&head, &mut head_vars)?;
        for term in &body {
            collect_variables(term, &mut body_vars)?;
        }
        if head_vars
            .iter()
            .any(|head_var| !body_vars.contains(head_var))
        {
            return Err(BfErr::ErrValue(E_INVARG.msg(
                "term_query() rule head variables must also appear in the body",
            )));
        }

        parsed.push(QueryRule { head, body });
    }

    Ok(parsed)
}

fn resolve_top_variable(
    term: &Var,
    bindings: &TermBindings,
    options: &TermOptions,
) -> Result<Var, BfErr> {
    let mut current = term.clone();
    let mut seen = Vec::new();
    loop {
        let Some(name) = variable_marker(&current)? else {
            return Ok(current);
        };
        if seen.contains(&name) {
            return Ok(current);
        }
        seen.push(name);
        let Some(bound) = bindings.get(&name) else {
            return Ok(current);
        };
        if seen.len() > options.max_depth {
            return Err(BfErr::ErrValue(
                E_MAXREC.msg("term_query() variable resolution exceeded max_depth"),
            ));
        }
        current = bound.clone();
    }
}

fn bind_query_variable(
    name: Symbol,
    value: &Var,
    bindings: &mut TermBindings,
    options: &TermOptions,
    depth: usize,
) -> Result<bool, BfErr> {
    let resolved = resolve_top_variable(value, bindings, options)?;
    if let Some(other_name) = variable_marker(&resolved)? {
        if other_name == name {
            return Ok(true);
        }
        if is_internal_query_var(other_name) && !is_internal_query_var(name) {
            return bind_query_variable(
                other_name,
                &variable_term(name),
                bindings,
                options,
                depth + 1,
            );
        }
    }

    if let Some(bound) = bindings.get(&name).cloned() {
        return term_query_match_inner(&bound, &resolved, bindings, options, depth + 1);
    }
    if bindings.len() >= options.max_bindings {
        return Err(BfErr::ErrValue(
            E_INVARG.msg("term_query() exceeded max_bindings"),
        ));
    }
    bindings.insert(name, resolved);
    Ok(true)
}

fn term_query_match_inner(
    left: &Var,
    right: &Var,
    bindings: &mut TermBindings,
    options: &TermOptions,
    depth: usize,
) -> Result<bool, BfErr> {
    check_depth(depth, options)?;

    let left = resolve_top_variable(left, bindings, options)?;
    let right = resolve_top_variable(right, bindings, options)?;

    if let Some(name) = variable_marker(&left)? {
        return bind_query_variable(name, &right, bindings, options, depth);
    }
    if let Some(name) = variable_marker(&right)? {
        return bind_query_variable(name, &left, bindings, options, depth);
    }

    match (left.variant(), right.variant()) {
        (Variant::List(left_list), Variant::List(right_list)) => {
            if left_list.len() != right_list.len() {
                return Ok(false);
            }
            for (left_item, right_item) in left_list.iter_ref().zip(right_list.iter_ref()) {
                if !term_query_match_inner(left_item, right_item, bindings, options, depth + 1)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        (Variant::Map(left_map), Variant::Map(right_map)) => {
            if left_map.len() != right_map.len() {
                return Ok(false);
            }
            for (key, left_value) in left_map.iter_ref() {
                let Some(right_value) = options.map_value(right_map, key) else {
                    return Ok(false);
                };
                if !term_query_match_inner(left_value, &right_value, bindings, options, depth + 1)?
                {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        _ => Ok(options.equal(&left, &right)),
    }
}

fn resolve_query_term(
    term: &Var,
    bindings: &TermBindings,
    options: &TermOptions,
    depth: usize,
) -> Result<Var, BfErr> {
    check_depth(depth, options)?;

    let resolved = resolve_top_variable(term, bindings, options)?;
    if variable_marker(&resolved)?.is_some() {
        return Ok(resolved);
    }

    match resolved.variant() {
        Variant::List(list) => {
            let mut changed_items = None::<Vec<Var>>;
            for (index, item) in list.iter_ref().enumerate() {
                let new_item = resolve_query_term(item, bindings, options, depth + 1)?;
                if let Some(items) = changed_items.as_mut() {
                    items.push(new_item);
                } else if new_item != *item {
                    let mut items = Vec::with_capacity(list.len());
                    items.extend(list.iter_ref().take(index).cloned());
                    items.push(new_item);
                    changed_items = Some(items);
                }
            }
            if let Some(items) = changed_items {
                Ok(v_list(&items))
            } else {
                Ok(resolved)
            }
        }
        Variant::Map(map) => {
            let mut changed_pairs = None::<Vec<(Var, Var)>>;
            for (index, (key, value)) in map.iter_ref().enumerate() {
                let new_value = resolve_query_term(value, bindings, options, depth + 1)?;
                if let Some(pairs) = changed_pairs.as_mut() {
                    pairs.push((key.clone(), new_value));
                } else if new_value != *value {
                    let mut pairs = Vec::with_capacity(map.len());
                    pairs.extend(
                        map.iter_ref()
                            .take(index)
                            .map(|(prev_key, prev_value)| (prev_key.clone(), prev_value.clone())),
                    );
                    pairs.push((key.clone(), new_value));
                    changed_pairs = Some(pairs);
                }
            }
            if let Some(pairs) = changed_pairs {
                Ok(v_map(&pairs))
            } else {
                Ok(resolved)
            }
        }
        _ => Ok(resolved),
    }
}

fn term_normalize_inner(
    term: &Var,
    bindings: &TermBindings,
    names: &mut TermBindings,
    options: &TermOptions,
    depth: usize,
) -> Result<Var, BfErr> {
    check_depth(depth, options)?;

    let resolved = resolve_top_variable(term, bindings, options)?;
    if let Some(name) = variable_marker(&resolved)? {
        if let Some(canonical) = names.get(&name) {
            return Ok(canonical.clone());
        }
        let canonical = variable_term(Symbol::mk(&format!("V{}", names.len() + 1)));
        names.insert(name, canonical.clone());
        return Ok(canonical);
    }

    match resolved.variant() {
        Variant::List(list) => {
            let mut items = Vec::with_capacity(list.len());
            for item in list.iter_ref() {
                items.push(term_normalize_inner(
                    item,
                    bindings,
                    names,
                    options,
                    depth + 1,
                )?);
            }
            Ok(v_list(&items))
        }
        Variant::Map(map) => {
            let mut pairs = Vec::with_capacity(map.len());
            for (key, value) in map.iter_ref() {
                pairs.push((
                    key.clone(),
                    term_normalize_inner(value, bindings, names, options, depth + 1)?,
                ));
            }
            Ok(v_map(&pairs))
        }
        _ => Ok(resolved),
    }
}

fn freshen_term(
    term: &Var,
    scope: usize,
    renames: &mut TermBindings,
    depth: usize,
    options: &TermOptions,
) -> Result<Var, BfErr> {
    check_depth(depth, options)?;

    if let Some(name) = variable_marker(term)? {
        if let Some(renamed) = renames.get(&name) {
            return Ok(renamed.clone());
        }
        let renamed_name = Symbol::mk(&format!(
            "{INTERNAL_QUERY_VAR_PREFIX}{scope}:{}",
            name.as_str()
        ));
        let renamed = variable_term(renamed_name);
        renames.insert(name, renamed.clone());
        return Ok(renamed);
    }

    match term.variant() {
        Variant::List(list) => {
            let mut changed_items = None::<Vec<Var>>;
            for (index, item) in list.iter_ref().enumerate() {
                let new_item = freshen_term(item, scope, renames, depth + 1, options)?;
                if let Some(items) = changed_items.as_mut() {
                    items.push(new_item);
                } else if new_item != *item {
                    let mut items = Vec::with_capacity(list.len());
                    items.extend(list.iter_ref().take(index).cloned());
                    items.push(new_item);
                    changed_items = Some(items);
                }
            }
            if let Some(items) = changed_items {
                Ok(v_list(&items))
            } else {
                Ok(term.clone())
            }
        }
        Variant::Map(map) => {
            let mut changed_pairs = None::<Vec<(Var, Var)>>;
            for (index, (key, value)) in map.iter_ref().enumerate() {
                let new_value = freshen_term(value, scope, renames, depth + 1, options)?;
                if let Some(pairs) = changed_pairs.as_mut() {
                    pairs.push((key.clone(), new_value));
                } else if new_value != *value {
                    let mut pairs = Vec::with_capacity(map.len());
                    pairs.extend(
                        map.iter_ref()
                            .take(index)
                            .map(|(prev_key, prev_value)| (prev_key.clone(), prev_value.clone())),
                    );
                    pairs.push((key.clone(), new_value));
                    changed_pairs = Some(pairs);
                }
            }
            if let Some(pairs) = changed_pairs {
                Ok(v_map(&pairs))
            } else {
                Ok(term.clone())
            }
        }
        _ => Ok(term.clone()),
    }
}

fn canonicalize_query_state_term(
    term: &Var,
    names: &mut TermBindings,
    options: &TermOptions,
    depth: usize,
) -> Result<Var, BfErr> {
    check_depth(depth, options)?;

    if let Some(name) = variable_marker(term)? {
        if !is_internal_query_var(name) {
            return Ok(term.clone());
        }
        if let Some(canonical) = names.get(&name) {
            return Ok(canonical.clone());
        }
        let canonical = variable_term(Symbol::mk(&format!("__term_query_state:{}", names.len())));
        names.insert(name, canonical.clone());
        return Ok(canonical);
    }

    match term.variant() {
        Variant::List(list) => {
            let mut items = Vec::with_capacity(list.len());
            for item in list.iter_ref() {
                items.push(canonicalize_query_state_term(
                    item,
                    names,
                    options,
                    depth + 1,
                )?);
            }
            Ok(v_list(&items))
        }
        Variant::Map(map) => {
            let mut pairs = Vec::with_capacity(map.len());
            for (key, value) in map.iter_ref() {
                pairs.push((
                    key.clone(),
                    canonicalize_query_state_term(value, names, options, depth + 1)?,
                ));
            }
            Ok(v_map(&pairs))
        }
        _ => Ok(term.clone()),
    }
}

/// Facts by their first and second elements, where those are scalars without variables:
/// equality on scalars is exactly what unification would test. Facts without such a key are
/// tried for every goal. Candidates keep the facts' order, so answers keep theirs.
#[derive(Default)]
struct FactIndex {
    by_first: HashMap<TermKey, Vec<usize>>,
    by_pair: HashMap<(TermKey, TermKey), Vec<usize>>,
    first_only: HashMap<TermKey, Vec<usize>>,
    unkeyed: Vec<usize>,
}

fn index_key(term: &Var, options: &TermOptions) -> Option<TermKey> {
    match term.variant() {
        Variant::List(_) | Variant::Map(_) | Variant::Flyweight(_) => None,
        _ => Some(options.key(term.clone())),
    }
}

fn indexed_elements(term: &Var, options: &TermOptions) -> (Option<TermKey>, Option<TermKey>) {
    let Variant::List(list) = term.variant() else {
        return (None, None);
    };
    let first = list
        .iter_ref()
        .next()
        .and_then(|item| index_key(item, options));
    let second = first
        .as_ref()
        .and(list.iter_ref().nth(1))
        .and_then(|item| index_key(item, options));
    (first, second)
}

impl FactIndex {
    fn new(facts: &[Var], options: &TermOptions) -> Self {
        let mut index = Self::default();
        for (position, fact) in facts.iter().enumerate() {
            match indexed_elements(fact, options) {
                (Some(first), second) => {
                    index
                        .by_first
                        .entry(first.clone())
                        .or_default()
                        .push(position);
                    match second {
                        Some(second) => index
                            .by_pair
                            .entry((first, second))
                            .or_default()
                            .push(position),
                        None => index.first_only.entry(first).or_default().push(position),
                    }
                }
                (None, _) => index.unkeyed.push(position),
            }
        }
        index
    }

    /// The facts that could match a goal whose leading elements are already resolved.
    fn candidates(&self, goal: &Var, count: usize, options: &TermOptions) -> Vec<usize> {
        let (first, second) = indexed_elements(goal, options);
        let Some(first) = first else {
            return (0..count).collect();
        };
        let mut candidates = self.unkeyed.clone();
        match second {
            Some(second) => {
                for group in [
                    self.by_pair.get(&(first.clone(), second)),
                    self.first_only.get(&first),
                ]
                .into_iter()
                .flatten()
                {
                    candidates.extend_from_slice(group);
                }
            }
            None => {
                if let Some(group) = self.by_first.get(&first) {
                    candidates.extend_from_slice(group);
                }
            }
        }
        candidates.sort_unstable();
        candidates
    }
}

/// A comparison goal, when `constraints` is on: `{'lt, a, b}` and its kin.
fn constraint_goal(goal: &Var) -> Option<(Symbol, Var, Var)> {
    let Variant::List(list) = goal.variant() else {
        return None;
    };
    if list.len() != 3 {
        return None;
    }
    let Variant::Sym(op) = list[0].variant() else {
        return None;
    };
    [*LT_SYM, *LE_SYM, *GT_SYM, *GE_SYM, *EQ_SYM, *NE_SYM]
        .contains(&op)
        .then(|| (op, list[1].clone(), list[2].clone()))
}

/// Compare as MOO's operators do: exact or MOO equality, and ordering only between like
/// types, with strings ordered by case when matching is case-sensitive.
fn constraint_holds(
    op: Symbol,
    left: &Var,
    right: &Var,
    options: &TermOptions,
) -> Result<bool, BfErr> {
    if op == *EQ_SYM {
        return Ok(options.equal(left, right));
    }
    if op == *NE_SYM {
        return Ok(!options.equal(left, right));
    }
    let mut ordering = left
        .compare(right)
        .map_err(|_| BfErr::ErrValue(E_TYPE.msg("term_query() cannot order these values")))?;
    if options.case_sensitive {
        ordering = left.cmp_case_sensitive(right);
    }
    Ok(if op == *LT_SYM {
        ordering.is_lt()
    } else if op == *LE_SYM {
        ordering.is_le()
    } else if op == *GT_SYM {
        ordering.is_gt()
    } else {
        ordering.is_ge()
    })
}

struct QueryEngine<'a> {
    facts: Vec<Var>,
    fact_ground: Vec<bool>,
    index: FactIndex,
    rules: Vec<QueryRule>,
    public_variables: &'a [Symbol],
    options: QueryOptions,
    tick_budget: Option<BuiltinTickBudget<'a>>,
    fresh_scope: usize,
    steps: usize,
    solutions: Vec<Var>,
    seen_solutions: HashSet<TermKey>,
    active_states: HashSet<TermKey>,
}

impl<'a> QueryEngine<'a> {
    fn new(
        facts: Vec<Var>,
        rules: Vec<QueryRule>,
        public_variables: &'a [Symbol],
        options: QueryOptions,
        tick_budget: Option<BuiltinTickBudget<'a>>,
    ) -> Result<Self, BfErr> {
        let fact_ground = facts
            .iter()
            .map(|fact| term_is_ground_inner(fact, &options.term, 0))
            .collect::<Result<Vec<_>, _>>()?;
        let index = FactIndex::new(&facts, &options.term);
        Ok(Self {
            facts,
            fact_ground,
            index,
            rules,
            public_variables,
            options,
            tick_budget,
            fresh_scope: 0,
            steps: 0,
            solutions: Vec::new(),
            seen_solutions: HashSet::new(),
            active_states: HashSet::new(),
        })
    }

    fn fresh_scope(&mut self) -> usize {
        let scope = self.fresh_scope;
        self.fresh_scope += 1;
        scope
    }

    fn check_search_step(&mut self) -> Result<(), BfErr> {
        self.steps += 1;
        if let Some(tick_budget) = self.tick_budget.as_mut() {
            tick_budget.consume_ticks(1)?;
        }
        if self.steps > self.options.max_steps {
            return Err(BfErr::ErrValue(
                E_MAXREC.msg("term_query() exceeded max_steps"),
            ));
        }
        Ok(())
    }

    fn project_solution(&self, bindings: &TermBindings) -> Result<Var, BfErr> {
        let mut pairs = Vec::with_capacity(self.public_variables.len());
        for name in self.public_variables {
            let resolved =
                resolve_query_term(&variable_term(*name), bindings, &self.options.term, 0)?;
            if variable_marker(&resolved)?.is_none() {
                pairs.push((v_sym(*name), resolved));
            }
        }
        Ok(v_map(&pairs))
    }

    fn query_state_key(&self, goals: &[Var], bindings: &TermBindings) -> Result<Var, BfErr> {
        let mut names = TermBindings::default();
        let mut resolved_goals = Vec::with_capacity(goals.len());
        for goal in goals {
            let resolved = resolve_query_term(goal, bindings, &self.options.term, 0)?;
            resolved_goals.push(canonicalize_query_state_term(
                &resolved,
                &mut names,
                &self.options.term,
                0,
            )?);
        }
        Ok(v_list(&resolved_goals))
    }

    fn cap(&self) -> usize {
        solution_cap(self.options.max_solutions, self.options.detail)
    }

    fn add_solution(&mut self, bindings: &TermBindings) -> Result<(), BfErr> {
        let solution = self.project_solution(bindings)?;
        if self.options.dedupe
            && !self
                .seen_solutions
                .insert(self.options.term.key(solution.clone()))
        {
            return Ok(());
        }
        self.solutions.push(solution);
        Ok(())
    }

    fn solve(
        &mut self,
        goals: Vec<Var>,
        bindings: TermBindings,
        depth: usize,
    ) -> Result<(), BfErr> {
        if self.solutions.len() >= self.cap() {
            return Ok(());
        }
        if depth > self.options.term.max_depth {
            return Err(BfErr::ErrValue(
                E_MAXREC.msg("term_query() exceeded max_depth"),
            ));
        }
        self.check_search_step()?;

        if goals.is_empty() {
            return self.add_solution(&bindings);
        }

        if self.options.constraints
            && let Some((op, left, right)) = constraint_goal(&goals[0])
        {
            let left = resolve_query_term(&left, &bindings, &self.options.term, 0)?;
            let right = resolve_query_term(&right, &bindings, &self.options.term, 0)?;
            if !term_is_ground_inner(&left, &self.options.term, 0)?
                || !term_is_ground_inner(&right, &self.options.term, 0)?
            {
                return Err(BfErr::ErrValue(E_INVARG.msg(format!(
                    "term_query() {op} needs bound values; bind them in an earlier goal"
                ))));
            }
            if constraint_holds(op, &left, &right, &self.options.term)? {
                self.solve(goals[1..].to_vec(), bindings, depth)?;
            }
            return Ok(());
        }

        let state_key = if self.options.dedupe {
            Some(
                self.options
                    .term
                    .key(self.query_state_key(&goals, &bindings)?),
            )
        } else {
            None
        };
        if let Some(key) = state_key.as_ref()
            && !self.active_states.insert(key.clone())
        {
            return Ok(());
        }

        let goal = &goals[0];
        let rest = goals[1..].to_vec();
        let resolved_goal = match goal.variant() {
            Variant::List(list) if !list.is_empty() => {
                let leading = list
                    .iter_ref()
                    .take(2)
                    .map(|item| resolve_top_variable(item, &bindings, &self.options.term))
                    .collect::<Result<Vec<_>, _>>()?;
                v_list(&leading)
            }
            _ => goal.clone(),
        };
        let candidates =
            self.index
                .candidates(&resolved_goal, self.facts.len(), &self.options.term);

        for fact_index in candidates {
            if self.solutions.len() >= self.cap() {
                break;
            }
            self.check_search_step()?;

            let fact = if self.fact_ground[fact_index] {
                self.facts[fact_index].clone()
            } else {
                let scope = self.fresh_scope();
                let mut renames = TermBindings::default();
                freshen_term(
                    &self.facts[fact_index],
                    scope,
                    &mut renames,
                    0,
                    &self.options.term,
                )?
            };
            let mut candidate_bindings = bindings.clone();
            if term_query_match_inner(goal, &fact, &mut candidate_bindings, &self.options.term, 0)?
            {
                self.solve(rest.clone(), candidate_bindings, depth)?;
            }
        }

        for rule_index in 0..self.rules.len() {
            if self.solutions.len() >= self.cap() {
                break;
            }
            self.check_search_step()?;

            let scope = self.fresh_scope();
            let mut renames = TermBindings::default();
            let rule = self.rules[rule_index].clone();
            let head = freshen_term(&rule.head, scope, &mut renames, 0, &self.options.term)?;
            let mut body = Vec::with_capacity(rule.body.len() + rest.len());
            for term in &rule.body {
                body.push(freshen_term(
                    term,
                    scope,
                    &mut renames,
                    0,
                    &self.options.term,
                )?);
            }
            body.extend(rest.iter().cloned());

            let mut candidate_bindings = bindings.clone();
            if term_query_match_inner(goal, &head, &mut candidate_bindings, &self.options.term, 0)?
            {
                self.solve(body, candidate_bindings, depth + 1)?;
            }
        }

        if let Some(key) = state_key {
            self.active_states.remove(&key);
        }
        Ok(())
    }
}

/// Usage: `list|map term_query(any query, list facts [, list rules [, map bindings [, map options]]])`
///
/// Evaluates a bounded positive-Horn query over caller-supplied terms. Rules
/// have shape `{head, body}` and bodies contain positive terms only. With the
/// `constraints` option, `{'lt, a, b}` and `le`, `gt`, `ge`, `eq`, `ne` goals
/// compare bound values.
fn bf_term_query(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.len() < 2 || bf_args.args.len() > 5 {
        return Err(BfErr::ErrValue(
            E_ARGS.msg("term_query() takes 2 to 5 arguments"),
        ));
    }

    let Some(facts) = bf_args.args[1].as_list() else {
        return Err(BfErr::ErrValue(
            E_TYPE.msg("term_query() facts must be a list"),
        ));
    };
    let facts = facts.iter_ref().cloned().collect::<Vec<_>>();
    let rules = if let Some(rules) = bf_args.args.iter_ref().nth(2) {
        parse_query_rules(rules)?
    } else {
        Vec::new()
    };
    let options = parse_query_options(bf_args.args.iter_ref().nth(4))?;
    let bindings = load_bindings(bf_args.args.iter_ref().nth(3), options.term.max_bindings)?;

    let mut public_variables = Vec::new();
    collect_variables(&bf_args.args[0], &mut public_variables)?;
    for name in bindings.keys() {
        if !public_variables.contains(name) {
            public_variables.push(*name);
        }
    }
    let query = bf_args.args[0].clone();

    let mut engine = QueryEngine::new(
        facts,
        rules,
        &public_variables,
        options,
        Some(bf_args.tick_budget()),
    )?;
    engine.solve(vec![query], bindings, 0)?;
    let QueryEngine {
        solutions, options, ..
    } = engine;
    Ok(Ret(search_result(
        solutions,
        options.max_solutions,
        options.detail,
        |complete| bf_args.v_bool(complete),
    )))
}

struct GridAstarPathArgs<'grid, 'budget> {
    width: usize,
    height: usize,
    start_x: i32,
    start_y: i32,
    goal_x: i32,
    goal_y: i32,
    passable: &'grid [bool],
    options: GridAstarOptions,
    tick_budget: Option<BuiltinTickBudget<'budget>>,
}

fn grid_astar_path(mut args: GridAstarPathArgs<'_, '_>) -> Result<GridAstarResult, BfErr> {
    let GridAstarPathArgs {
        width,
        height,
        start_x,
        start_y,
        goal_x,
        goal_y,
        passable,
        options,
        ref mut tick_budget,
    } = args;

    let w = width as i32;
    let h = height as i32;

    // Validate start/goal in bounds.
    if start_x < 0 || start_x >= w || start_y < 0 || start_y >= h {
        return Err(BfErr::ErrValue(
            E_INVARG.msg("start position out of bounds"),
        ));
    }
    if goal_x < 0 || goal_x >= w || goal_y < 0 || goal_y >= h {
        return Err(BfErr::ErrValue(E_INVARG.msg("goal position out of bounds")));
    }
    let grid_size = width * height;

    // Check goal is passable.
    if !passable[(goal_y as usize) * width + (goal_x as usize)] {
        return Ok(GridAstarResult {
            path: Vec::new(),
            cost: -1,
            visited: 0,
        });
    }

    // Already there.
    if start_x == goal_x && start_y == goal_y {
        return Ok(GridAstarResult {
            path: Vec::new(),
            cost: 0,
            visited: 0,
        });
    }

    let is_passable = |x: i32, y: i32| -> bool {
        x >= 0 && x < w && y >= 0 && y < h && passable[(y as usize) * width + (x as usize)]
    };

    // A* with binary heap (min-heap via Reverse).
    // Node: (f_score, g_score, x, y)
    let mut open: BinaryHeap<Reverse<(i32, i32, i32, i32)>> = BinaryHeap::new();
    let mut g_score = vec![i32::MAX; grid_size];
    let mut came_from = vec![u32::MAX; grid_size]; // flat index of parent

    let start_idx = (start_y as usize) * width + (start_x as usize);
    let goal_idx = (goal_y as usize) * width + (goal_x as usize);

    g_score[start_idx] = 0;
    let h0 = (goal_x - start_x).abs().max((goal_y - start_y).abs()); // Chebyshev
    open.push(Reverse((h0, 0, start_x, start_y)));
    let mut visited = 0;

    // 8-directional neighbors.
    const DIRS: [(i32, i32); 8] = [
        (-1, -1),
        (0, -1),
        (1, -1),
        (-1, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
    ];

    while let Some(Reverse((_f, g, cx, cy))) = open.pop() {
        if let Some(tick_budget) = tick_budget.as_mut() {
            tick_budget.consume_ticks(1)?;
        }
        if let Some(max_nodes) = options.max_nodes
            && visited >= max_nodes
        {
            return Err(BfErr::ErrValue(
                E_MAXREC.msg("grid_astar() exceeded max_nodes"),
            ));
        }
        visited += 1;

        let cidx = (cy as usize) * width + (cx as usize);

        // Skip stale entries.
        if g > g_score[cidx] {
            continue;
        }

        // Goal reached.
        if cidx == goal_idx {
            // Reconstruct path.
            let mut path = Vec::new();
            let mut idx = goal_idx;
            while idx != start_idx {
                let px = (idx % width) as i64;
                let py = (idx / width) as i64;
                path.push(v_list(&[v_int(px), v_int(py)]));
                idx = came_from[idx] as usize;
            }
            path.reverse();
            return Ok(GridAstarResult {
                path,
                cost: g as i64,
                visited,
            });
        }

        let ng = g + 1;

        for &(dx, dy) in &DIRS {
            if options.directions == 4 && dx != 0 && dy != 0 {
                continue;
            }
            let nx = cx + dx;
            let ny = cy + dy;

            if !is_passable(nx, ny) {
                continue;
            }

            // Diagonal corner-cutting check.
            if !options.corner_cutting
                && dx != 0
                && dy != 0
                && (!is_passable(cx + dx, cy) || !is_passable(cx, cy + dy))
            {
                continue;
            }

            let nidx = (ny as usize) * width + (nx as usize);
            if ng < g_score[nidx] {
                g_score[nidx] = ng;
                came_from[nidx] = cidx as u32;
                let heuristic = (goal_x - nx).abs().max((goal_y - ny).abs());
                open.push(Reverse((ng + heuristic, ng, nx, ny)));
            }
        }
    }

    // No path found.
    Ok(GridAstarResult {
        path: Vec::new(),
        cost: -1,
        visited,
    })
}

fn grid_coord(x: i32, y: i32) -> Var {
    v_list(&[v_int(x as i64), v_int(y as i64)])
}

fn grid_line_points(
    width: usize,
    height: usize,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    mut tick_budget: Option<BuiltinTickBudget<'_>>,
) -> Result<Vec<(i32, i32)>, BfErr> {
    check_grid_position(width, height, x0, y0, "start")?;
    check_grid_position(width, height, x1, y1, "end")?;

    let mut points = Vec::new();
    let mut x = x0;
    let mut y = y0;
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        if let Some(tick_budget) = tick_budget.as_mut() {
            tick_budget.consume_ticks(1)?;
        }
        points.push((x, y));
        if x == x1 && y == y1 {
            return Ok(points);
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x += sx;
        }
        if e2 <= dx {
            err += dx;
            y += sy;
        }
    }
}

fn grid_index(width: usize, x: i32, y: i32) -> usize {
    y as usize * width + x as usize
}

fn grid_is_passable(width: usize, height: usize, passable: &[bool], x: i32, y: i32) -> bool {
    x >= 0 && x < width as i32 && y >= 0 && y < height as i32 && passable[grid_index(width, x, y)]
}

fn grid_neighbors(
    width: usize,
    height: usize,
    passable: &[bool],
    x: i32,
    y: i32,
    options: GridOptions,
    out: &mut Vec<(i32, i32)>,
) {
    const DIRS: [(i32, i32); 8] = [
        (-1, -1),
        (0, -1),
        (1, -1),
        (-1, 0),
        (1, 0),
        (-1, 1),
        (0, 1),
        (1, 1),
    ];

    out.clear();
    for &(dx, dy) in &DIRS {
        if options.directions == 4 && dx != 0 && dy != 0 {
            continue;
        }
        let nx = x + dx;
        let ny = y + dy;
        if !grid_is_passable(width, height, passable, nx, ny) {
            continue;
        }
        if !options.corner_cutting
            && dx != 0
            && dy != 0
            && (!grid_is_passable(width, height, passable, x + dx, y)
                || !grid_is_passable(width, height, passable, x, y + dy))
        {
            continue;
        }
        out.push((nx, ny));
    }
}

fn grid_flood_points(
    width: usize,
    height: usize,
    start: (i32, i32),
    passable: &[bool],
    options: GridOptions,
    include_start: bool,
    mut tick_budget: Option<BuiltinTickBudget<'_>>,
) -> Result<Vec<Var>, BfErr> {
    let (start_x, start_y) = start;
    check_grid_position(width, height, start_x, start_y, "start")?;
    if !grid_is_passable(width, height, passable, start_x, start_y) {
        return Ok(Vec::new());
    }

    let mut seen = vec![false; width * height];
    let mut queue = VecDeque::new();
    let mut neighbors = Vec::with_capacity(8);
    let mut result = Vec::new();
    seen[grid_index(width, start_x, start_y)] = true;
    queue.push_back((start_x, start_y));
    let mut visited = 0;

    while let Some((x, y)) = queue.pop_front() {
        if let Some(tick_budget) = tick_budget.as_mut() {
            tick_budget.consume_ticks(1)?;
        }
        if let Some(max_nodes) = options.max_nodes
            && visited >= max_nodes
        {
            return Err(BfErr::ErrValue(
                E_MAXREC.msg("grid flood exceeded max_nodes"),
            ));
        }
        visited += 1;
        if include_start || x != start_x || y != start_y {
            result.push(grid_coord(x, y));
        }

        grid_neighbors(width, height, passable, x, y, options, &mut neighbors);
        for &(nx, ny) in &neighbors {
            let idx = grid_index(width, nx, ny);
            if seen[idx] {
                continue;
            }
            seen[idx] = true;
            queue.push_back((nx, ny));
        }
    }

    Ok(result)
}

fn graph_edge(edge: &Var) -> Result<(Var, Var), BfErr> {
    match edge.variant() {
        Variant::List(list) if list.len() == 2 => Ok((list[0].clone(), list[1].clone())),
        Variant::List(list) if list.len() >= 3 => {
            if matches!(list[0].variant(), Variant::Sym(sym) if sym == *EDGE_SYM) {
                Ok((list[1].clone(), list[2].clone()))
            } else {
                Err(BfErr::ErrValue(E_INVARG.msg(
                    "graph edges must be {from, to}, {'edge, from, to}, or ['from -> from, 'to -> to]",
                )))
            }
        }
        Variant::Map(map) => {
            let from = map
                .get(&v_sym(*FROM_SYM))
                .map_err(|_| BfErr::ErrValue(E_INVARG.msg("graph edge map missing 'from key")))?;
            let to = map
                .get(&v_sym(*TO_SYM))
                .map_err(|_| BfErr::ErrValue(E_INVARG.msg("graph edge map missing 'to key")))?;
            Ok((from, to))
        }
        _ => Err(BfErr::ErrValue(E_INVARG.msg(
            "graph edges must be {from, to}, {'edge, from, to}, or ['from -> from, 'to -> to]",
        ))),
    }
}

fn graph_add_node(nodes: &mut Vec<Var>, seen: &mut HashSet<Var>, node: &Var) {
    if seen.insert(node.clone()) {
        nodes.push(node.clone());
    }
}

fn parse_graph(edges: &Var) -> Result<Graph, BfErr> {
    let Some(edges) = edges.as_list() else {
        return Err(BfErr::ErrValue(E_TYPE.msg("graph edges must be a list")));
    };

    let mut nodes = Vec::new();
    let mut seen = HashSet::new();
    let mut adjacency = HashMap::<Var, Vec<Var>>::default();
    let mut indegree = HashMap::<Var, usize>::default();
    for edge in edges.iter_ref() {
        let (from, to) = graph_edge(edge)?;
        graph_add_node(&mut nodes, &mut seen, &from);
        graph_add_node(&mut nodes, &mut seen, &to);
        let targets = adjacency.entry(from.clone()).or_default();
        if !targets.contains(&to) {
            targets.push(to.clone());
            *indegree.entry(to.clone()).or_default() += 1;
        }
        indegree.entry(from).or_default();
    }

    Ok(Graph {
        nodes,
        adjacency,
        indegree,
    })
}

fn graph_check_step(
    steps: &mut usize,
    visited: usize,
    options: &GraphOptions,
    tick_budget: &mut Option<BuiltinTickBudget<'_>>,
) -> Result<(), BfErr> {
    if let Some(tick_budget) = tick_budget.as_mut() {
        tick_budget.consume_ticks(1)?;
    }
    *steps += 1;
    if *steps > options.max_steps {
        return Err(BfErr::ErrValue(
            E_MAXREC.msg("graph builtin exceeded max_steps"),
        ));
    }
    if let Some(max_nodes) = options.max_nodes
        && visited >= max_nodes
    {
        return Err(BfErr::ErrValue(
            E_MAXREC.msg("graph builtin exceeded max_nodes"),
        ));
    }
    Ok(())
}

fn graph_shortest_path(
    graph: &Graph,
    start: &Var,
    goal: &Var,
    options: &GraphOptions,
    mut tick_budget: Option<BuiltinTickBudget<'_>>,
) -> Result<Vec<Var>, BfErr> {
    if start == goal {
        return Ok(vec![start.clone()]);
    }

    let mut queue = VecDeque::new();
    let mut seen = HashSet::new();
    let mut parent = HashMap::<Var, Var>::default();
    let mut steps = 0usize;
    queue.push_back(start.clone());
    seen.insert(start.clone());

    while let Some(node) = queue.pop_front() {
        graph_check_step(&mut steps, seen.len(), options, &mut tick_budget)?;
        let Some(targets) = graph.adjacency.get(&node) else {
            continue;
        };
        for target in targets {
            if !seen.insert(target.clone()) {
                continue;
            }
            parent.insert(target.clone(), node.clone());
            if target == goal {
                let mut path = vec![goal.clone()];
                let mut current = goal.clone();
                while let Some(prev) = parent.get(&current) {
                    path.push(prev.clone());
                    if prev == start {
                        break;
                    }
                    current = prev.clone();
                }
                path.reverse();
                return Ok(path);
            }
            queue.push_back(target.clone());
        }
    }

    Ok(Vec::new())
}

fn graph_reachable(
    graph: &Graph,
    start: &Var,
    options: &GraphOptions,
    mut tick_budget: Option<BuiltinTickBudget<'_>>,
) -> Result<Vec<Var>, BfErr> {
    let mut queue = VecDeque::new();
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    let mut steps = 0usize;
    queue.push_back(start.clone());
    seen.insert(start.clone());

    while let Some(node) = queue.pop_front() {
        graph_check_step(&mut steps, seen.len(), options, &mut tick_budget)?;
        let Some(targets) = graph.adjacency.get(&node) else {
            continue;
        };
        for target in targets {
            if !seen.insert(target.clone()) {
                continue;
            }
            result.push(target.clone());
            queue.push_back(target.clone());
        }
    }

    Ok(result)
}

fn graph_topsort(
    graph: &Graph,
    options: &GraphOptions,
    mut tick_budget: Option<BuiltinTickBudget<'_>>,
) -> Result<Vec<Var>, BfErr> {
    let mut indegree = graph.indegree.clone();
    let mut queue = VecDeque::new();
    let mut result = Vec::with_capacity(graph.nodes.len());
    let mut steps = 0usize;

    for node in &graph.nodes {
        if indegree.get(node).copied().unwrap_or_default() == 0 {
            queue.push_back(node.clone());
        }
    }

    while let Some(node) = queue.pop_front() {
        graph_check_step(&mut steps, result.len(), options, &mut tick_budget)?;
        result.push(node.clone());
        if let Some(targets) = graph.adjacency.get(&node) {
            for target in targets {
                let Some(count) = indegree.get_mut(target) else {
                    continue;
                };
                *count -= 1;
                if *count == 0 {
                    queue.push_back(target.clone());
                }
            }
        }
    }

    if result.len() != graph.nodes.len() {
        return Err(BfErr::ErrValue(
            E_INVARG.msg("graph_topsort() graph contains a cycle"),
        ));
    }
    Ok(result)
}

/// Usage: `list|int|map grid_astar(int width, int height, int start_x, int start_y, int goal_x, int goal_y, list tile_map, list solid_tiles [, map options])`
///
/// A* pathfinding on a tile grid. Returns a list of `{x, y}` waypoints from
/// the start to the goal (excluding the start position), or an empty list if
/// no path exists.
///
/// Supports 8-directional movement (cardinal + diagonal). Diagonal moves are
/// only permitted when both adjacent cardinal tiles are passable (no corner-cutting).
///
/// `tile_map` is a flat list of tile IDs (1-based indexing, row-major).
/// `solid_tiles` is a list of tile IDs that are impassable.
///
/// Uses Chebyshev distance as the heuristic (diagonal cost = cardinal cost).
fn bf_grid_astar(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.len() < 8 || bf_args.args.len() > 9 {
        return Err(BfErr::ErrValue(
            E_ARGS.msg("grid_astar() takes 8 or 9 arguments"),
        ));
    }

    let width = match bf_args.args[0].variant() {
        Variant::Int(i) if i > 0 => i as usize,
        _ => {
            return Err(BfErr::ErrValue(
                E_INVARG.msg("width must be a positive integer"),
            ));
        }
    };
    let height = match bf_args.args[1].variant() {
        Variant::Int(i) if i > 0 => i as usize,
        _ => {
            return Err(BfErr::ErrValue(
                E_INVARG.msg("height must be a positive integer"),
            ));
        }
    };
    let start_x = match bf_args.args[2].variant() {
        Variant::Int(i) => i as i32,
        _ => return Err(BfErr::ErrValue(E_TYPE.msg("start_x must be an integer"))),
    };
    let start_y = match bf_args.args[3].variant() {
        Variant::Int(i) => i as i32,
        _ => return Err(BfErr::ErrValue(E_TYPE.msg("start_y must be an integer"))),
    };
    let goal_x = match bf_args.args[4].variant() {
        Variant::Int(i) => i as i32,
        _ => return Err(BfErr::ErrValue(E_TYPE.msg("goal_x must be an integer"))),
    };
    let goal_y = match bf_args.args[5].variant() {
        Variant::Int(i) => i as i32,
        _ => return Err(BfErr::ErrValue(E_TYPE.msg("goal_y must be an integer"))),
    };

    let tile_map_list = bf_args.args[6]
        .as_list()
        .ok_or_else(|| BfErr::ErrValue(E_TYPE.msg("tile_map must be a list")))?;
    let solid_tiles_list = bf_args.args[7]
        .as_list()
        .ok_or_else(|| BfErr::ErrValue(E_TYPE.msg("solid_tiles must be a list")))?;
    let options = parse_grid_astar_options(bf_args.args.iter_ref().nth(8))?;

    let grid_size = width * height;
    let mut solid_ids = std::collections::HashSet::new();
    for item in solid_tiles_list.iter() {
        if let Variant::Int(id) = item.variant() {
            solid_ids.insert(id);
        }
    }

    let mut passable = vec![true; grid_size];
    for (i, item) in tile_map_list.iter().enumerate() {
        if i >= grid_size {
            break;
        }
        if let Variant::Int(tile_id) = item.variant()
            && solid_ids.contains(&tile_id)
        {
            passable[i] = false;
        }
    }

    let result = grid_astar_path(GridAstarPathArgs {
        width,
        height,
        start_x,
        start_y,
        goal_x,
        goal_y,
        passable: &passable,
        options,
        tick_budget: Some(bf_args.tick_budget()),
    })?;
    let value = match options.return_mode {
        GridAstarReturn::Path => v_list_iter(result.path),
        GridAstarReturn::Cost => v_int(result.cost),
        GridAstarReturn::Detail => v_map(&[
            (v_sym(*PATH_SYM), v_list_iter(result.path)),
            (v_sym(*COST_SYM), v_int(result.cost)),
            (v_sym(*VISITED_SYM), v_int(result.visited as i64)),
            (v_sym(*FOUND_SYM), bf_args.v_bool(result.cost >= 0)),
        ]),
    };
    Ok(Ret(value))
}

/// Usage: `list grid_line(int width, int height, int x0, int y0, int x1, int y1)`
///
/// Returns the inclusive Bresenham line between two in-bounds grid coordinates.
fn bf_grid_line(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.len() != 6 {
        return Err(BfErr::ErrValue(E_ARGS.msg("grid_line() takes 6 arguments")));
    }

    let width = positive_dimension(&bf_args.args[0], "width")?;
    let height = positive_dimension(&bf_args.args[1], "height")?;
    let x0 = integer_coordinate(&bf_args.args[2], "x0")?;
    let y0 = integer_coordinate(&bf_args.args[3], "y0")?;
    let x1 = integer_coordinate(&bf_args.args[4], "x1")?;
    let y1 = integer_coordinate(&bf_args.args[5], "y1")?;

    let points = grid_line_points(width, height, x0, y0, x1, y1, Some(bf_args.tick_budget()))?
        .into_iter()
        .map(|(x, y)| grid_coord(x, y))
        .collect::<Vec<_>>();
    Ok(Ret(v_list_iter(points)))
}

/// Usage: `bool grid_los(int width, int height, int x0, int y0, int x1, int y1, list tile_map, list solid_tiles)`
///
/// Tests line of sight over a tile grid using the inclusive Bresenham line.
fn bf_grid_los(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.len() != 8 {
        return Err(BfErr::ErrValue(E_ARGS.msg("grid_los() takes 8 arguments")));
    }

    let width = positive_dimension(&bf_args.args[0], "width")?;
    let height = positive_dimension(&bf_args.args[1], "height")?;
    let x0 = integer_coordinate(&bf_args.args[2], "x0")?;
    let y0 = integer_coordinate(&bf_args.args[3], "y0")?;
    let x1 = integer_coordinate(&bf_args.args[4], "x1")?;
    let y1 = integer_coordinate(&bf_args.args[5], "y1")?;
    let passable = parse_passable_grid(width, height, &bf_args.args[6], &bf_args.args[7])?;
    let points = grid_line_points(width, height, x0, y0, x1, y1, Some(bf_args.tick_budget()))?;

    let clear = points
        .into_iter()
        .all(|(x, y)| grid_is_passable(width, height, &passable, x, y));
    Ok(Ret(bf_args.v_bool(clear)))
}

/// Usage: `list grid_reachable(int width, int height, int start_x, int start_y, list tile_map, list solid_tiles [, map options])`
///
/// Returns reachable coordinates from a start tile, excluding the start tile.
fn bf_grid_reachable(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.len() < 6 || bf_args.args.len() > 7 {
        return Err(BfErr::ErrValue(
            E_ARGS.msg("grid_reachable() takes 6 or 7 arguments"),
        ));
    }

    let width = positive_dimension(&bf_args.args[0], "width")?;
    let height = positive_dimension(&bf_args.args[1], "height")?;
    let start_x = integer_coordinate(&bf_args.args[2], "start_x")?;
    let start_y = integer_coordinate(&bf_args.args[3], "start_y")?;
    let passable = parse_passable_grid(width, height, &bf_args.args[4], &bf_args.args[5])?;
    let options = parse_grid_options(bf_args.args.iter_ref().nth(6))?;
    let points = grid_flood_points(
        width,
        height,
        (start_x, start_y),
        &passable,
        options,
        false,
        Some(bf_args.tick_budget()),
    )?;
    Ok(Ret(v_list_iter(points)))
}

/// Usage: `list grid_flood(int width, int height, int start_x, int start_y, list tile_map, list solid_tiles [, map options])`
///
/// Returns the flood-filled region from a start tile, including the start tile.
fn bf_grid_flood(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.len() < 6 || bf_args.args.len() > 7 {
        return Err(BfErr::ErrValue(
            E_ARGS.msg("grid_flood() takes 6 or 7 arguments"),
        ));
    }

    let width = positive_dimension(&bf_args.args[0], "width")?;
    let height = positive_dimension(&bf_args.args[1], "height")?;
    let start_x = integer_coordinate(&bf_args.args[2], "start_x")?;
    let start_y = integer_coordinate(&bf_args.args[3], "start_y")?;
    let passable = parse_passable_grid(width, height, &bf_args.args[4], &bf_args.args[5])?;
    let options = parse_grid_options(bf_args.args.iter_ref().nth(6))?;
    let points = grid_flood_points(
        width,
        height,
        (start_x, start_y),
        &passable,
        options,
        true,
        Some(bf_args.tick_budget()),
    )?;
    Ok(Ret(v_list_iter(points)))
}

/// Usage: `list graph_shortest_path(list edges, any from, any to [, map options])`
///
/// Returns an unweighted shortest node path, including the start and goal nodes.
fn bf_graph_shortest_path(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.len() < 3 || bf_args.args.len() > 4 {
        return Err(BfErr::ErrValue(
            E_ARGS.msg("graph_shortest_path() takes 3 or 4 arguments"),
        ));
    }

    let graph = parse_graph(&bf_args.args[0])?;
    let options = parse_graph_options(bf_args.args.iter_ref().nth(3))?;
    let path = graph_shortest_path(
        &graph,
        &bf_args.args[1],
        &bf_args.args[2],
        &options,
        Some(bf_args.tick_budget()),
    )?;
    Ok(Ret(v_list(&path)))
}

/// Usage: `list graph_reachable(list edges, any from [, map options])`
///
/// Returns nodes reachable from the supplied start node, excluding the start node.
fn bf_graph_reachable(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.len() < 2 || bf_args.args.len() > 3 {
        return Err(BfErr::ErrValue(
            E_ARGS.msg("graph_reachable() takes 2 or 3 arguments"),
        ));
    }

    let graph = parse_graph(&bf_args.args[0])?;
    let options = parse_graph_options(bf_args.args.iter_ref().nth(2))?;
    let reachable = graph_reachable(
        &graph,
        &bf_args.args[1],
        &options,
        Some(bf_args.tick_budget()),
    )?;
    Ok(Ret(v_list(&reachable)))
}

/// Usage: `list graph_topsort(list edges [, map options])`
///
/// Returns a deterministic topological ordering for an acyclic directed graph.
fn bf_graph_topsort(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.is_empty() || bf_args.args.len() > 2 {
        return Err(BfErr::ErrValue(
            E_ARGS.msg("graph_topsort() takes 1 or 2 arguments"),
        ));
    }

    let graph = parse_graph(&bf_args.args[0])?;
    let options = parse_graph_options(bf_args.args.iter_ref().nth(1))?;
    let sorted = graph_topsort(&graph, &options, Some(bf_args.tick_budget()))?;
    Ok(Ret(v_list(&sorted)))
}

static OCTAVES_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("octaves"));
static FREQUENCY_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("frequency"));
static PERSISTENCE_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("persistence"));
static LACUNARITY_SYM: LazyLock<Symbol> = LazyLock::new(|| Symbol::mk("lacunarity"));

/// The most points one `noise_grid()` call may sample.
const MAX_NOISE_GRID_POINTS: usize = 65_536;

fn noise_coordinate(value: &Var, name: &str) -> Result<f64, BfErr> {
    value
        .as_float_numeric()
        .filter(|n| n.is_finite())
        .ok_or_else(|| BfErr::ErrValue(E_TYPE.msg(format!("{name} must be a number"))))
}

fn noise_seed(value: &Var) -> Result<i64, BfErr> {
    value
        .as_integer()
        .ok_or_else(|| BfErr::ErrValue(E_TYPE.msg("seed must be an integer")))
}

fn parse_noise_options(value: Option<&Var>, function: &str) -> Result<noise::Octaves, BfErr> {
    let mut options = noise::Octaves::default();
    let Some(value) = value else {
        return Ok(options);
    };
    let Some(map) = value.as_map() else {
        return Err(BfErr::ErrValue(
            E_TYPE.msg(format!("{function}() options must be a map")),
        ));
    };
    for (key, value) in map.iter_ref() {
        let key = option_key_symbol(key)?;
        if key == *OCTAVES_SYM {
            match value.as_integer() {
                Some(n) if (1..=i64::from(noise::MAX_OCTAVES)).contains(&n) => {
                    options.octaves = n as u32;
                }
                Some(_) => {
                    return Err(BfErr::ErrValue(E_INVARG.msg(format!(
                        "octaves option must be from 1 to {}",
                        noise::MAX_OCTAVES
                    ))));
                }
                None => {
                    return Err(BfErr::ErrValue(
                        E_TYPE.msg("octaves option must be an integer"),
                    ));
                }
            }
            continue;
        }
        let slot = if key == *FREQUENCY_SYM {
            &mut options.frequency
        } else if key == *PERSISTENCE_SYM {
            &mut options.persistence
        } else if key == *LACUNARITY_SYM {
            &mut options.lacunarity
        } else {
            return Err(BfErr::ErrValue(
                E_INVARG.msg(format!("unknown {function}() option: {key}")),
            ));
        };
        let n = value
            .as_float_numeric()
            .ok_or_else(|| BfErr::ErrValue(E_TYPE.msg(format!("{key} option must be a number"))))?;
        if !n.is_finite() || n <= 0.0 {
            return Err(BfErr::ErrValue(
                E_INVARG.msg(format!("{key} option must be a positive number")),
            ));
        }
        *slot = n;
    }
    Ok(options)
}

/// Usage: `float noise_2d(num x, num y, int seed [, map options])`
///
/// Returns smooth, seeded 2D simplex noise at (x, y), from -1.0 to 1.0. Nearby points
/// give nearby values, and the same arguments always give the same result, on every
/// platform and in every release.
///
/// Options: `octaves` (1 to 16, default 1) layers finer noise over coarser; `frequency`
/// (default 1.0) scales the coordinates; each further octave multiplies the frequency by
/// `lacunarity` (default 2.0) and its weight by `persistence` (default 0.5).
fn bf_noise_2d(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.len() < 3 || bf_args.args.len() > 4 {
        return Err(BfErr::ErrValue(
            E_ARGS.msg("noise_2d() takes 3 or 4 arguments"),
        ));
    }
    let x = noise_coordinate(&bf_args.args[0], "x")?;
    let y = noise_coordinate(&bf_args.args[1], "y")?;
    let seed = noise_seed(&bf_args.args[2])?;
    let options = parse_noise_options(bf_args.args.iter_ref().nth(3), "noise_2d")?;
    let noise = noise::Simplex::new(seed);
    Ok(Ret(v_float(options.sample(&noise, x, y))))
}

/// Usage: `list noise_grid(num x, num y, int width, int height, int seed [, map options])`
///
/// Returns `noise_2d()` sampled over a grid in one call: a list of `height` rows, each a
/// list of `width` values, where row r, column c is the noise at (x + c, y + r). Takes the
/// same options as `noise_2d()`. At most 65,536 points.
fn bf_noise_grid(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.len() < 5 || bf_args.args.len() > 6 {
        return Err(BfErr::ErrValue(
            E_ARGS.msg("noise_grid() takes 5 or 6 arguments"),
        ));
    }
    let x = noise_coordinate(&bf_args.args[0], "x")?;
    let y = noise_coordinate(&bf_args.args[1], "y")?;
    let width = positive_dimension(&bf_args.args[2], "width")?;
    let height = positive_dimension(&bf_args.args[3], "height")?;
    if width.saturating_mul(height) > MAX_NOISE_GRID_POINTS {
        return Err(BfErr::ErrValue(E_INVARG.msg(format!(
            "noise_grid() samples at most {MAX_NOISE_GRID_POINTS} points"
        ))));
    }
    let seed = noise_seed(&bf_args.args[4])?;
    let options = parse_noise_options(bf_args.args.iter_ref().nth(5), "noise_grid")?;
    let noise = noise::Simplex::new(seed);
    let mut budget = bf_args.tick_budget();
    let mut rows = Vec::with_capacity(height);
    for row in 0..height {
        budget.consume_ticks(1)?;
        let row_y = y + row as f64;
        rows.push(v_list_iter((0..width).map(|col| {
            v_float(options.sample(&noise, x + col as f64, row_y))
        })));
    }
    Ok(Ret(v_list_iter(rows)))
}

/// Usage: `int random_at(int seed, list coordinates [, int max])`
///
/// Returns an integer from 1 to `max` (default the largest integer) fixed by the seed and
/// a list of integer coordinates: like `random()`, but repeatable. The same arguments
/// always give the same result, on every platform and in every release, and the order of
/// the coordinates matters.
fn bf_random_at(bf_args: &mut BfCallState<'_>) -> Result<BfRet, BfErr> {
    if bf_args.args.len() < 2 || bf_args.args.len() > 3 {
        return Err(BfErr::ErrValue(
            E_ARGS.msg("random_at() takes 2 or 3 arguments"),
        ));
    }
    let seed = noise_seed(&bf_args.args[0])?;
    let Some(list) = bf_args.args[1].as_list() else {
        return Err(BfErr::ErrValue(
            E_TYPE.msg("random_at() coordinates must be a list"),
        ));
    };
    let mut coordinates = Vec::with_capacity(list.len());
    for item in list.iter() {
        let Some(n) = item.as_integer() else {
            return Err(BfErr::ErrValue(
                E_TYPE.msg("random_at() coordinates must be integers"),
            ));
        };
        coordinates.push(n);
    }
    let max = match bf_args.args.iter_ref().nth(2) {
        None => i64::MAX,
        Some(value) => match value.as_integer() {
            Some(max) if max >= 1 => max,
            Some(_) => {
                return Err(BfErr::ErrValue(
                    E_INVARG.msg("random_at() max must be at least 1"),
                ));
            }
            None => {
                return Err(BfErr::ErrValue(
                    E_TYPE.msg("random_at() max must be an integer"),
                ));
            }
        },
    };
    Ok(Ret(v_int(noise::random_at(seed, &coordinates, max))))
}

pub(crate) fn register_bf_algorithms(builtins: &mut [BuiltinFunction]) {
    builtins[offset_for_builtin("grid_astar")] = bf_grid_astar;
    builtins[offset_for_builtin("noise_2d")] = bf_noise_2d;
    builtins[offset_for_builtin("noise_grid")] = bf_noise_grid;
    builtins[offset_for_builtin("random_at")] = bf_random_at;
    builtins[offset_for_builtin("grid_reachable")] = bf_grid_reachable;
    builtins[offset_for_builtin("grid_line")] = bf_grid_line;
    builtins[offset_for_builtin("grid_los")] = bf_grid_los;
    builtins[offset_for_builtin("grid_flood")] = bf_grid_flood;
    builtins[offset_for_builtin("term_unify")] = bf_term_unify;
    builtins[offset_for_builtin("term_match")] = bf_term_match;
    builtins[offset_for_builtin("term_substitute")] = bf_term_substitute;
    builtins[offset_for_builtin("term_query")] = bf_term_query;
    builtins[offset_for_builtin("term_variables")] = bf_term_variables;
    builtins[offset_for_builtin("term_ground")] = bf_term_ground;
    builtins[offset_for_builtin("term_normalize")] = bf_term_normalize;
    builtins[offset_for_builtin("graph_shortest_path")] = bf_graph_shortest_path;
    builtins[offset_for_builtin("graph_reachable")] = bf_graph_reachable;
    builtins[offset_for_builtin("graph_topsort")] = bf_graph_topsort;
}

#[cfg(test)]
mod tests {
    use moor_var::{Associative, v_bool, v_empty_list, v_float, v_int, v_str};

    use super::*;

    fn var(name: &str) -> Var {
        v_list(&[v_sym("var"), v_sym(name)])
    }

    fn unify(pattern: Var, datum: Var) -> Result<Option<Var>, BfErr> {
        let options = TermOptions::default();
        let mut bindings = TermBindings::default();
        if term_unify_inner(&pattern, &datum, &mut bindings, &options, 0)? {
            Ok(Some(bindings_to_var(bindings)))
        } else {
            Ok(None)
        }
    }

    fn term(name: &str, values: &[Var]) -> Var {
        let mut items = Vec::with_capacity(values.len() + 1);
        items.push(v_sym(name));
        items.extend(values.iter().cloned());
        v_list(&items)
    }

    fn query(query: Var, facts: Vec<Var>, rules: Vec<Var>) -> Result<Var, BfErr> {
        query_with_bindings(query, facts, rules, TermBindings::default())
    }

    fn query_with_bindings(
        query: Var,
        facts: Vec<Var>,
        rules: Vec<Var>,
        bindings: TermBindings,
    ) -> Result<Var, BfErr> {
        query_with_options(query, facts, rules, bindings, QueryOptions::default())
    }

    fn query_with_options(
        query: Var,
        facts: Vec<Var>,
        rules: Vec<Var>,
        bindings: TermBindings,
        options: QueryOptions,
    ) -> Result<Var, BfErr> {
        let rule_list = v_list(&rules);
        let parsed_rules = parse_query_rules(&rule_list)?;
        let mut public_variables = Vec::new();
        collect_variables(&query, &mut public_variables)?;
        for name in bindings.keys() {
            if !public_variables.contains(name) {
                public_variables.push(*name);
            }
        }
        let mut engine = QueryEngine::new(facts, parsed_rules, &public_variables, options, None)?;
        engine.solve(vec![query], bindings, 0)?;
        Ok(v_list(&engine.solutions))
    }

    #[test]
    fn grid_astar_consumes_task_tick_budget() {
        let passable = vec![true; 25];
        let mut tick_count = 0;
        let err = grid_astar_path(GridAstarPathArgs {
            width: 5,
            height: 5,
            start_x: 0,
            start_y: 0,
            goal_x: 4,
            goal_y: 4,
            passable: &passable,
            options: GridAstarOptions::default(),
            tick_budget: Some(BuiltinTickBudget::new(&mut tick_count, 2)),
        })
        .unwrap_err();

        assert!(matches!(err, BfErr::ErrValue(error) if error.err_type() == E_MAXREC));
        assert_eq!(tick_count, 2);
    }

    #[test]
    fn grid_astar_can_restrict_to_cardinal_movement() {
        let passable = vec![true; 9];
        let result = grid_astar_path(GridAstarPathArgs {
            width: 3,
            height: 3,
            start_x: 0,
            start_y: 0,
            goal_x: 2,
            goal_y: 2,
            passable: &passable,
            options: GridAstarOptions {
                directions: 4,
                ..GridAstarOptions::default()
            },
            tick_budget: None,
        })
        .unwrap();

        assert_eq!(result.cost, 4);
        assert_eq!(result.path.len(), 4);
    }

    #[test]
    fn grid_astar_respects_max_nodes_option() {
        let passable = vec![true; 25];
        let err = grid_astar_path(GridAstarPathArgs {
            width: 5,
            height: 5,
            start_x: 0,
            start_y: 0,
            goal_x: 4,
            goal_y: 4,
            passable: &passable,
            options: GridAstarOptions {
                max_nodes: Some(2),
                ..GridAstarOptions::default()
            },
            tick_budget: None,
        })
        .unwrap_err();

        assert!(matches!(err, BfErr::ErrValue(error) if error.err_type() == E_MAXREC));
    }

    #[test]
    fn grid_line_returns_inclusive_bresenham_points() {
        let points = grid_line_points(5, 5, 0, 0, 4, 2, None).unwrap();
        assert_eq!(points, vec![(0, 0), (1, 1), (2, 1), (3, 2), (4, 2)]);
    }

    #[test]
    fn grid_los_line_detects_solid_tile() {
        let passable = vec![true, true, true, true, false, true, true, true, true];
        let points = grid_line_points(3, 3, 0, 0, 2, 2, None).unwrap();
        let clear = points
            .into_iter()
            .all(|(x, y)| grid_is_passable(3, 3, &passable, x, y));

        assert!(!clear);
    }

    #[test]
    fn grid_flood_respects_cardinal_movement() {
        let passable = vec![true, false, true, false, true, false, true, false, true];
        let points = grid_flood_points(
            3,
            3,
            (1, 1),
            &passable,
            GridOptions {
                directions: 4,
                ..GridOptions::default()
            },
            true,
            None,
        )
        .unwrap();

        assert_eq!(points, vec![grid_coord(1, 1)]);
    }

    #[test]
    fn graph_shortest_path_returns_bfs_path() {
        let graph = parse_graph(&v_list(&[
            v_list(&[v_sym("A"), v_sym("B")]),
            v_list(&[v_sym("B"), v_sym("C")]),
            v_list(&[v_sym("A"), v_sym("C")]),
        ]))
        .unwrap();
        let path = graph_shortest_path(
            &graph,
            &v_sym("A"),
            &v_sym("C"),
            &GraphOptions::default(),
            None,
        )
        .unwrap();

        assert_eq!(path, vec![v_sym("A"), v_sym("C")]);
    }

    #[test]
    fn graph_reachable_uses_input_edge_order() {
        let graph = parse_graph(&v_list(&[
            v_list(&[v_sym("A"), v_sym("B")]),
            v_list(&[v_sym("A"), v_sym("C")]),
            v_list(&[v_sym("B"), v_sym("D")]),
        ]))
        .unwrap();
        let reachable =
            graph_reachable(&graph, &v_sym("A"), &GraphOptions::default(), None).unwrap();

        assert_eq!(reachable, vec![v_sym("B"), v_sym("C"), v_sym("D")]);
    }

    #[test]
    fn graph_topsort_rejects_cycles() {
        let graph = parse_graph(&v_list(&[
            v_list(&[v_sym("A"), v_sym("B")]),
            v_list(&[v_sym("B"), v_sym("A")]),
        ]))
        .unwrap();
        let err = graph_topsort(&graph, &GraphOptions::default(), None).unwrap_err();

        assert!(matches!(err, BfErr::ErrValue(error) if error.err_type() == E_INVARG));
    }

    #[test]
    fn term_query_consumes_task_tick_budget() {
        let query = var("X");
        let facts = vec![v_int(1)];
        let mut public_variables = Vec::new();
        collect_variables(&query, &mut public_variables).unwrap();

        let mut tick_count = 0;
        let err = {
            let mut engine = QueryEngine::new(
                facts,
                Vec::new(),
                &public_variables,
                QueryOptions::default(),
                Some(BuiltinTickBudget::new(&mut tick_count, 2)),
            )
            .unwrap();
            engine.solve(vec![query], TermBindings::default(), 0)
        }
        .unwrap_err();

        assert!(matches!(err, BfErr::ErrValue(error) if error.err_type() == E_MAXREC));
        assert_eq!(tick_count, 2);
    }

    #[test]
    fn term_unify_binds_nested_variables() {
        let result = unify(
            v_list(&[v_sym("edge"), var("From"), v_list(&[var("To"), v_int(9)])]),
            v_list(&[v_sym("edge"), v_int(10), v_list(&[v_int(11), v_int(9)])]),
        )
        .unwrap()
        .unwrap();
        let bindings = result.as_map().unwrap();
        assert_eq!(bindings.get(&v_sym("From")).unwrap(), v_int(10));
        assert_eq!(bindings.get(&v_sym("To")).unwrap(), v_int(11));
    }

    #[test]
    fn term_unify_repeated_variable_must_match() {
        assert!(
            unify(
                v_list(&[var("X"), v_list(&[var("x")])]),
                v_list(&[v_str("same"), v_list(&[v_str("SAME")])]),
            )
            .unwrap()
            .is_some()
        );
        assert!(
            unify(
                v_list(&[var("X"), v_list(&[var("x")])]),
                v_list(&[v_int(1), v_list(&[v_int(2)])]),
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn term_unify_respects_existing_bindings() {
        let options = TermOptions::default();
        let mut bindings = TermBindings::default();
        bindings.insert(Symbol::mk("From"), v_int(10));
        let matched = term_unify_inner(
            &v_list(&[v_sym("edge"), var("from"), var("To")]),
            &v_list(&[v_sym("edge"), v_int(10), v_int(11)]),
            &mut bindings,
            &options,
            0,
        )
        .unwrap();
        assert!(matched);
        assert_eq!(bindings.get(&Symbol::mk("To")).unwrap(), &v_int(11));

        let mut bindings = TermBindings::default();
        bindings.insert(Symbol::mk("From"), v_int(12));
        let matched = term_unify_inner(
            &v_list(&[v_sym("edge"), var("from")]),
            &v_list(&[v_sym("edge"), v_int(10)]),
            &mut bindings,
            &options,
            0,
        )
        .unwrap();
        assert!(!matched);
    }

    #[test]
    fn term_unify_matches_map_values_by_exact_key_set() {
        let pattern = v_map(&[(v_sym("edge"), v_list(&[var("From"), var("To")]))]);
        let datum = v_map(&[(v_sym("EDGE"), v_list(&[v_int(1), v_int(2)]))]);
        let result = unify(pattern, datum).unwrap().unwrap();
        let bindings = result.as_map().unwrap();
        assert_eq!(bindings.get(&v_sym("from")).unwrap(), v_int(1));
        assert_eq!(bindings.get(&v_sym("to")).unwrap(), v_int(2));

        let pattern = v_map(&[(v_sym("edge"), var("X"))]);
        let datum = v_map(&[(v_sym("edge"), v_int(1)), (v_sym("extra"), v_int(2))]);
        assert!(unify(pattern, datum).unwrap().is_none());
    }

    #[test]
    fn term_unify_rejects_malformed_variable_marker() {
        let err = unify(v_list(&[v_sym("var"), v_str("X")]), v_int(1)).unwrap_err();
        assert!(matches!(err, BfErr::ErrValue(error) if error.err_type() == E_INVARG));
    }

    #[test]
    fn term_match_returns_successful_bindings() {
        let results = term_match_values(
            &v_list(&[v_sym("edge"), v_int(1), var("To")]),
            &[
                v_list(&[v_sym("edge"), v_int(1), v_int(2)]),
                v_list(&[v_sym("edge"), v_int(1), v_int(2)]),
                v_list(&[v_sym("edge"), v_int(2), v_int(3)]),
            ],
            &[TermBindings::default()],
            &MatchOptions::default(),
            None,
        )
        .unwrap();

        assert_eq!(results, vec![v_map(&[(v_sym("To"), v_int(2))])]);
    }

    #[test]
    fn term_matching_case_sensitive_values_and_keys() {
        let options = MatchOptions {
            term: TermOptions {
                case_sensitive: true,
                ..TermOptions::default()
            },
            ..MatchOptions::default()
        };
        let values = [v_str("Key"), v_str("key"), v_str("Key")];
        let exact = term_match_values(
            &var("X"),
            &values,
            &[TermBindings::default()],
            &options,
            None,
        )
        .unwrap();
        assert_eq!(exact.len(), 2);
        assert!(exact[0].eq_case_sensitive(&v_map(&[(v_sym("X"), v_str("Key"))])));
        assert!(exact[1].eq_case_sensitive(&v_map(&[(v_sym("X"), v_str("key"))])));
        let ordinary = term_match_values(
            &var("X"),
            &values,
            &[TermBindings::default()],
            &MatchOptions::default(),
            None,
        )
        .unwrap();
        assert_eq!(ordinary.len(), 1);

        for (pattern, datum) in [
            (
                v_list(&[var("X"), var("X")]),
                v_list(&[v_str("Key"), v_str("key")]),
            ),
            (
                v_map(&[(v_str("Key"), var("X"))]),
                v_map(&[(v_str("key"), v_int(1))]),
            ),
            (v_list(&[v_str("Key")]), v_list(&[v_str("key")])),
        ] {
            assert!(
                !term_unify_inner(
                    &pattern,
                    &datum,
                    &mut TermBindings::default(),
                    &options.term,
                    0
                )
                .unwrap()
            );
        }
        let mut initial = TermBindings::default();
        initial.insert(Symbol::mk("X"), v_str("Key"));
        assert!(
            !term_unify_inner(&var("X"), &v_str("key"), &mut initial, &options.term, 0).unwrap()
        );
    }

    #[test]
    fn term_match_deduplicates_equal_float_zero_signs() {
        let values = [
            v_map(&[(v_sym("value"), v_float(0.0))]),
            v_map(&[(v_sym("value"), v_float(-0.0))]),
        ];
        for case_sensitive in [false, true] {
            let options = MatchOptions {
                term: TermOptions {
                    case_sensitive,
                    ..TermOptions::default()
                },
                ..MatchOptions::default()
            };
            let results = term_match_values(
                &var("X"),
                &values,
                &[TermBindings::default()],
                &options,
                None,
            )
            .unwrap();
            assert_eq!(results.len(), 1);
        }
    }

    #[test]
    fn term_query_case_sensitive_recursive_states_and_answers() {
        let rules = vec![
            v_list(&[
                term("reach", &[var("X"), var("Y")]),
                v_list(&[term("edge", &[var("X"), var("Y")])]),
            ]),
            v_list(&[
                term("reach", &[var("X"), var("Y")]),
                v_list(&[
                    term("edge", &[var("X"), var("Z")]),
                    term("reach", &[var("Z"), var("Y")]),
                ]),
            ]),
        ];
        let result = query_with_options(
            term("reach", &[v_str("a"), var("To")]),
            vec![
                term("edge", &[v_str("a"), v_str("A")]),
                term("edge", &[v_str("A"), v_str("end")]),
            ],
            rules,
            TermBindings::default(),
            QueryOptions {
                term: TermOptions {
                    case_sensitive: true,
                    ..TermOptions::default()
                },
                ..QueryOptions::default()
            },
        )
        .unwrap();
        assert!(result.eq_case_sensitive(&v_list(&[
            v_map(&[(v_sym("To"), v_str("A"))]),
            v_map(&[(v_sym("To"), v_str("end"))]),
        ])));
        let answers = query_with_options(
            var("X"),
            vec![v_str("a"), v_str("A"), v_str("a")],
            vec![],
            TermBindings::default(),
            QueryOptions {
                term: TermOptions {
                    case_sensitive: true,
                    ..TermOptions::default()
                },
                ..QueryOptions::default()
            },
        )
        .unwrap();
        assert_eq!(answers.as_list().unwrap().len(), 2);
    }

    #[test]
    fn term_match_respects_max_steps() {
        let options = MatchOptions {
            max_steps: 1,
            ..MatchOptions::default()
        };
        let err = term_match_values(
            &var("X"),
            &[v_int(1), v_int(2)],
            &[TermBindings::default()],
            &options,
            None,
        )
        .unwrap_err();

        assert!(matches!(err, BfErr::ErrValue(error) if error.err_type() == E_MAXREC));
    }

    #[test]
    fn term_variables_reports_unique_names_in_traversal_order() {
        let mut variables = Vec::new();
        collect_variables(
            &v_list(&[
                var("From"),
                v_map(&[(v_sym("nested"), v_list(&[var("To"), var("From")]))]),
            ]),
            &mut variables,
        )
        .unwrap();

        assert_eq!(variables, vec![Symbol::mk("From"), Symbol::mk("To")]);
    }

    #[test]
    fn term_ground_detects_variable_markers() {
        let options = TermOptions::default();
        assert!(term_is_ground_inner(&v_list(&[v_sym("edge"), v_int(1)]), &options, 0).unwrap());
        assert!(!term_is_ground_inner(&v_list(&[v_sym("edge"), var("X")]), &options, 0).unwrap());
    }

    #[test]
    fn term_normalize_resolves_bindings_and_canonicalizes_variables() {
        let mut bindings = TermBindings::default();
        bindings.insert(Symbol::mk("X"), v_int(10));
        bindings.insert(Symbol::mk("Y"), var("Z"));
        let mut names = TermBindings::default();
        let normalized = term_normalize_inner(
            &v_list(&[var("Y"), var("X"), var("Z")]),
            &bindings,
            &mut names,
            &TermOptions::default(),
            0,
        )
        .unwrap();

        assert_eq!(normalized, v_list(&[var("V1"), v_int(10), var("V1")]));
    }

    #[test]
    fn term_query_matches_nested_variables_in_one_fact() {
        let result = query(
            v_list(&[v_sym("edge"), var("From"), v_list(&[var("To"), v_int(9)])]),
            vec![v_list(&[
                v_sym("edge"),
                v_int(10),
                v_list(&[v_int(11), v_int(9)]),
            ])],
            vec![],
        )
        .unwrap();
        let bindings = result.as_list().unwrap()[0].as_map().unwrap();
        assert_eq!(bindings.get(&v_sym("From")).unwrap(), v_int(10));
        assert_eq!(bindings.get(&v_sym("To")).unwrap(), v_int(11));
    }

    #[test]
    fn term_query_repeated_variable_must_match() {
        assert_eq!(
            query(
                v_list(&[var("X"), v_list(&[var("x")])]),
                vec![v_list(&[v_str("same"), v_list(&[v_str("SAME")])])],
                vec![],
            )
            .unwrap(),
            v_list(&[v_map(&[(v_sym("X"), v_str("same"))])])
        );
        assert_eq!(
            query(
                v_list(&[var("X"), v_list(&[var("x")])]),
                vec![v_list(&[v_int(1), v_list(&[v_int(2)])])],
                vec![],
            )
            .unwrap(),
            v_empty_list()
        );
    }

    #[test]
    fn term_query_respects_existing_bindings() {
        let mut bindings = TermBindings::default();
        bindings.insert(Symbol::mk("From"), v_int(10));
        let result = query_with_bindings(
            v_list(&[v_sym("edge"), var("from"), var("To")]),
            vec![v_list(&[v_sym("edge"), v_int(10), v_int(11)])],
            vec![],
            bindings,
        )
        .unwrap();
        assert_eq!(
            result,
            v_list(&[v_map(&[
                (v_sym("from"), v_int(10)),
                (v_sym("To"), v_int(11)),
            ])])
        );

        let mut bindings = TermBindings::default();
        bindings.insert(Symbol::mk("From"), v_int(12));
        let result = query_with_bindings(
            v_list(&[v_sym("edge"), var("from")]),
            vec![v_list(&[v_sym("edge"), v_int(10)])],
            vec![],
            bindings,
        )
        .unwrap();
        assert_eq!(result, v_list(&[]));
    }

    #[test]
    fn term_query_matches_map_values_by_exact_key_set() {
        let result = query(
            v_map(&[(v_sym("edge"), v_list(&[var("From"), var("To")]))]),
            vec![v_map(&[(v_sym("EDGE"), v_list(&[v_int(1), v_int(2)]))])],
            vec![],
        )
        .unwrap();
        let bindings = result.as_list().unwrap()[0].as_map().unwrap();
        assert_eq!(bindings.get(&v_sym("from")).unwrap(), v_int(1));
        assert_eq!(bindings.get(&v_sym("to")).unwrap(), v_int(2));

        let pattern = v_map(&[(v_sym("edge"), var("X"))]);
        let datum = v_map(&[(v_sym("edge"), v_int(1)), (v_sym("extra"), v_int(2))]);
        assert_eq!(query(pattern, vec![datum], vec![]).unwrap(), v_empty_list());
    }

    #[test]
    fn term_query_rejects_malformed_variable_marker() {
        let err = query(v_list(&[v_sym("var"), v_str("X")]), vec![v_int(1)], vec![]).unwrap_err();
        assert!(matches!(err, BfErr::ErrValue(error) if error.err_type() == E_INVARG));
    }

    #[test]
    fn term_substitute_replaces_nested_variables() {
        let mut bindings = TermBindings::default();
        bindings.insert(Symbol::mk("Target"), v_int(123));
        let template = v_list(&[
            v_str("property_write"),
            var("target"),
            v_map(&[(v_sym("name"), var("Target"))]),
        ]);
        let (result, changed) =
            term_substitute_inner(&template, &bindings, &TermOptions::default(), 0).unwrap();
        assert!(changed);
        assert_eq!(
            result,
            v_list(&[
                v_str("property_write"),
                v_int(123),
                v_map(&[(v_sym("name"), v_int(123))])
            ])
        );
    }

    #[test]
    fn term_substitute_can_leave_unbound_variables() {
        let options = TermOptions {
            unbound: UnboundMode::Leave,
            ..TermOptions::default()
        };
        let template = v_list(&[v_int(1), var("Missing")]);
        let (result, changed) =
            term_substitute_inner(&template, &TermBindings::default(), &options, 0).unwrap();
        assert!(!changed);
        assert_eq!(result, template);
    }

    #[test]
    fn term_query_matches_facts_in_input_order() {
        let result = query(
            term("edge", &[v_int(1), var("To")]),
            vec![
                term("edge", &[v_int(1), v_int(2)]),
                term("edge", &[v_int(1), v_int(3)]),
                term("edge", &[v_int(2), v_int(4)]),
            ],
            vec![],
        )
        .unwrap();

        assert_eq!(
            result,
            v_list(&[
                v_map(&[(v_sym("To"), v_int(2))]),
                v_map(&[(v_sym("To"), v_int(3))])
            ])
        );
    }

    #[test]
    fn term_query_allows_recursive_positive_rules() {
        let reachable = |from: Var, to: Var| term("reachable", &[from, to]);
        let edge = |from: Var, to: Var| term("edge", &[from, to]);
        let facts = vec![
            edge(v_int(1), v_int(2)),
            edge(v_int(2), v_int(3)),
            edge(v_int(3), v_int(4)),
        ];
        let rules = vec![
            v_list(&[
                reachable(var("A"), var("B")),
                v_list(&[edge(var("A"), var("B"))]),
            ]),
            v_list(&[
                reachable(var("A"), var("B")),
                v_list(&[edge(var("A"), var("C")), reachable(var("C"), var("B"))]),
            ]),
        ];

        let result = query(reachable(v_int(1), var("Where")), facts, rules).unwrap();
        assert_eq!(
            result,
            v_list(&[
                v_map(&[(v_sym("Where"), v_int(2))]),
                v_map(&[(v_sym("Where"), v_int(3))]),
                v_map(&[(v_sym("Where"), v_int(4))]),
            ])
        );
    }

    #[test]
    fn term_query_dedupes_cyclic_recursive_solutions() {
        let reachable = |from: Var, to: Var| term("reachable", &[from, to]);
        let edge = |from: Var, to: Var| term("edge", &[from, to]);
        let facts = vec![edge(v_int(1), v_int(2)), edge(v_int(2), v_int(1))];
        let rules = vec![
            v_list(&[
                reachable(var("A"), var("B")),
                v_list(&[edge(var("A"), var("B"))]),
            ]),
            v_list(&[
                reachable(var("A"), var("B")),
                v_list(&[edge(var("A"), var("C")), reachable(var("C"), var("B"))]),
            ]),
        ];

        let result = query(reachable(v_int(1), var("Where")), facts, rules).unwrap();
        assert_eq!(
            result,
            v_list(&[
                v_map(&[(v_sym("Where"), v_int(2))]),
                v_map(&[(v_sym("Where"), v_int(1))]),
            ])
        );
    }

    #[test]
    fn term_query_rejects_unsafe_rules() {
        let rules = v_list(&[v_list(&[
            term("reachable", &[var("A"), var("B")]),
            v_list(&[term("edge", &[var("A"), var("C")])]),
        ])]);
        let err = parse_query_rules(&rules).unwrap_err();
        assert!(matches!(err, BfErr::ErrValue(error) if error.err_type() == E_INVARG));
    }

    #[test]
    fn term_query_rejects_negative_body_terms() {
        let rules = v_list(&[v_list(&[
            term("reachable", &[var("A"), var("B")]),
            v_list(&[term("not", &[term("blocked", &[var("A"), var("B")])])]),
        ])]);
        let err = parse_query_rules(&rules).unwrap_err();
        assert!(matches!(err, BfErr::ErrValue(error) if error.err_type() == E_INVARG));
    }

    fn solutions(result: Var) -> Vec<Var> {
        result.as_list().unwrap().iter_ref().cloned().collect()
    }

    fn query_options(configure: impl FnOnce(&mut QueryOptions)) -> QueryOptions {
        let mut options = QueryOptions::default();
        configure(&mut options);
        options
    }

    #[test]
    fn term_match_takes_each_binding_set_in_order() {
        let values = [
            term("near", &[v_int(1), v_int(10)]),
            term("near", &[v_int(2), v_int(20)]),
            term("near", &[v_int(1), v_int(30)]),
        ];
        let sets = [
            TermBindings::from_iter([(Symbol::mk("A"), v_int(2))]),
            TermBindings::from_iter([(Symbol::mk("A"), v_int(1))]),
            TermBindings::from_iter([(Symbol::mk("A"), v_int(2))]),
        ];
        let results = term_match_values(
            &term("near", &[var("A"), var("S")]),
            &values,
            &sets,
            &MatchOptions::default(),
            None,
        )
        .unwrap();
        let scores: Vec<Var> = results
            .iter()
            .map(|row| row.as_map().unwrap().get(&v_sym("S")).unwrap())
            .collect();
        assert_eq!(
            scores,
            vec![v_int(20), v_int(10), v_int(30)],
            "set order, then value order; the repeated set adds nothing"
        );
    }

    #[test]
    fn detail_reports_whether_the_limit_cut_the_search() {
        let values: Vec<Var> = (1..=3).map(v_int).collect();
        for (max_solutions, complete) in [(2, false), (3, true), (4, true)] {
            let options = MatchOptions {
                max_solutions,
                detail: true,
                ..MatchOptions::default()
            };
            let rows = term_match_values(
                &var("X"),
                &values,
                &[TermBindings::default()],
                &options,
                None,
            )
            .unwrap();
            let result = search_result(rows, max_solutions, true, v_bool);
            let map = result.as_map().unwrap();
            assert_eq!(
                map.get(&v_sym("complete")).unwrap(),
                v_bool(complete),
                "limit {max_solutions}"
            );
            assert_eq!(
                map.get(&v_sym("rows")).unwrap().as_list().unwrap().len(),
                max_solutions.min(3)
            );
        }
        let facts: Vec<Var> = (1..=3).map(|i| term("n", &[v_int(i)])).collect();
        let options = query_options(|options| {
            options.max_solutions = 3;
            options.detail = true;
        });
        let found = query_with_options(
            term("n", &[var("X")]),
            facts,
            vec![],
            TermBindings::default(),
            options,
        )
        .unwrap();
        assert_eq!(
            solutions(found).len(),
            3,
            "with detail the engine looks one answer past the limit, and finds none"
        );
    }

    #[test]
    fn indexed_queries_find_what_a_full_scan_finds() {
        let facts = vec![
            term("edge", &[v_int(1), v_int(2)]),
            term("edge", &[var("Any"), v_int(9)]),
            term("edge", &[v_int(1), v_int(3)]),
            v_list(&[var("F"), v_int(1), v_int(4)]),
            term("edge", &[v_str("Room"), v_int(5)]),
            term("edge", &[v_int(2), v_int(6)]),
            v_int(7),
            term("edge", &[v_list(&[v_int(1)]), v_int(8)]),
        ];
        let expected = |from: Var| {
            vec![v_int(2), v_int(9), v_int(3), v_int(4)]
                .into_iter()
                .filter(|_| from == v_int(1))
                .collect::<Vec<_>>()
        };
        let found =
            solutions(query(term("edge", &[v_int(1), var("To")]), facts.clone(), vec![]).unwrap());
        let tos: Vec<Var> = found
            .iter()
            .map(|row| row.as_map().unwrap().get(&v_sym("To")).unwrap())
            .collect();
        assert_eq!(
            tos,
            expected(v_int(1)),
            "keyed, unkeyed and wildcard facts, in fact order"
        );

        let room = solutions(
            query(
                term("edge", &[v_str("room"), var("To")]),
                facts.clone(),
                vec![],
            )
            .unwrap(),
        );
        assert_eq!(
            room.len(),
            2,
            "case-insensitive keys still find \"Room\", and the wildcard edge"
        );
        let exact = query_with_options(
            term("edge", &[v_str("room"), var("To")]),
            facts.clone(),
            vec![],
            TermBindings::default(),
            query_options(|options| options.term.case_sensitive = true),
        )
        .unwrap();
        assert_eq!(solutions(exact).len(), 1, "case-sensitive keys do not");

        let open = solutions(
            query(
                term("edge", &[var("From"), v_int(8)]),
                facts.clone(),
                vec![],
            )
            .unwrap(),
        );
        assert_eq!(open.len(), 1, "an unbound first argument scans the facts");

        let rules = vec![v_list(&[
            term("two_step", &[var("A"), var("C")]),
            v_list(&[
                term("edge", &[var("A"), var("B")]),
                term("edge", &[var("B"), var("C")]),
            ]),
        ])];
        let joined =
            solutions(query(term("two_step", &[v_int(1), var("C")]), facts, rules).unwrap());
        let ends: Vec<Var> = joined
            .iter()
            .map(|row| row.as_map().unwrap().get(&v_sym("C")).unwrap())
            .collect();
        assert!(
            ends.contains(&v_int(6)),
            "a join through the index binds the second goal from the first: {ends:?}"
        );
    }

    #[test]
    fn constraints_compare_bound_values() {
        let facts: Vec<Var> = [5, 2, 9]
            .into_iter()
            .map(|score| term("score", &[v_int(score)]))
            .collect();
        let rules = vec![v_list(&[
            term("low", &[var("S")]),
            v_list(&[
                term("score", &[var("S")]),
                term("lt", &[var("S"), v_int(6)]),
            ]),
        ])];
        let on = query_options(|options| options.constraints = true);
        let low = query_with_options(
            term("low", &[var("S")]),
            facts.clone(),
            rules.clone(),
            TermBindings::default(),
            on,
        )
        .unwrap();
        let scores: Vec<Var> = solutions(low)
            .iter()
            .map(|row| row.as_map().unwrap().get(&v_sym("S")).unwrap())
            .collect();
        assert_eq!(scores, vec![v_int(5), v_int(2)]);

        let off = query(term("low", &[var("S")]), facts.clone(), rules).unwrap();
        assert!(
            solutions(off).is_empty(),
            "without the option, lt is an ordinary goal with no facts"
        );

        let unbound = vec![v_list(&[
            term("early", &[var("S")]),
            v_list(&[
                term("lt", &[var("S"), v_int(6)]),
                term("score", &[var("S")]),
            ]),
        ])];
        let err = query_with_options(
            term("early", &[var("S")]),
            facts.clone(),
            unbound,
            TermBindings::default(),
            on,
        )
        .unwrap_err();
        assert!(
            matches!(err, BfErr::ErrValue(ref error) if error.err_type() == E_INVARG),
            "comparing an unbound value is an error"
        );

        let mixed = vec![v_list(&[
            term("bad", &[var("S")]),
            v_list(&[
                term("score", &[var("S")]),
                term("lt", &[var("S"), v_str("6")]),
            ]),
        ])];
        let err = query_with_options(
            term("bad", &[var("S")]),
            facts,
            mixed,
            TermBindings::default(),
            on,
        )
        .unwrap_err();
        assert!(
            matches!(err, BfErr::ErrValue(ref error) if error.err_type() == E_TYPE),
            "only like types order"
        );

        let case = |case_sensitive| {
            let options = query_options(|options| {
                options.constraints = true;
                options.term.case_sensitive = case_sensitive;
            });
            let found = query_with_options(
                term("eq", &[v_str("a"), v_str("A")]),
                vec![],
                vec![],
                TermBindings::default(),
                options,
            )
            .unwrap();
            !solutions(found).is_empty()
        };
        assert!(case(false), "eq follows MOO equality");
        assert!(!case(true), "and exact equality when case-sensitive");
    }
}
