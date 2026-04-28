; Tree-sitter query for Go symbol extraction.
;
; Captures named declarations across multiple patterns. Each pattern uses a
; consistent prefix so the Rust dispatcher identifies the symbol kind by
; which captures are present.
;
; Go visibility is determined by the FIRST CHARACTER of the identifier
; (uppercase = exported, lowercase = package-private). Filtering happens
; in Rust after capture.

;; Top-level function: `func Foo(...) { ... }`
(function_declaration
  name: (identifier) @function.name
  body: (block) @function.body) @function.def

;; Method: `func (r *Foo) Bar(...) { ... }`. The `receiver` parameter list is
;; captured so we can derive the parent type name in Rust.
(method_declaration
  receiver: (parameter_list) @method.receiver
  name: (field_identifier) @method.name
  body: (block) @method.body) @method.def

;; Type declaration: struct, interface, or named type. We capture the inner
;; type node (`type.kind`) so Rust can dispatch on `node.kind() == "struct_type"`,
;; `"interface_type"`, etc. The `type.def` is the inner `type_spec` (NOT the
;; outer `type_declaration`) so grouped declarations like
;;   type ( Foo struct {}; Bar interface {} )
;; produce one symbol per spec rather than collapsing into the outer node.
(type_declaration
  (type_spec
    name: (type_identifier) @type.name
    type: _ @type.kind) @type.def)

;; Type alias: `type Foo = Bar`
(type_declaration
  (type_alias
    name: (type_identifier) @typealias.name) @typealias.def)

;; Const: `const Foo = ...` (and grouped `const ( ... )` blocks)
(const_declaration
  (const_spec
    name: (identifier) @const.name) @const.def)

;; Var: `var Foo = ...` (and grouped `var ( ... )` blocks)
(var_declaration
  (var_spec
    name: (identifier) @const.name) @const.def)
