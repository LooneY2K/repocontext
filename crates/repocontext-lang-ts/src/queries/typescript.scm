; Tree-sitter query for TypeScript/TSX symbol extraction.
;
; Captures named symbols across multiple patterns. Each pattern uses a
; consistent prefix (function/class/interface/typealias/enum/const/method/field)
; so the Rust dispatcher identifies the symbol kind by which captures are present.
;
; The `*.def` capture is always on the OUTER `export_statement` for top-level
; exports (so signatures and doc-comment lookup include the `export` keyword and
; any preceding JSDoc). For class members, `*.def` is on the member node itself.

;; Exported function: `export function foo(...) { ... }` (incl. async)
(export_statement
  (function_declaration
    name: (identifier) @function.name
    body: (statement_block) @function.body)) @function.def

;; Exported class: `export class Foo { ... }`
(export_statement
  (class_declaration
    name: (type_identifier) @class.name
    body: (class_body) @class.body)) @class.def

;; Exported abstract class: `export abstract class Foo { ... }`
(export_statement
  (abstract_class_declaration
    name: (type_identifier) @class.name
    body: (class_body) @class.body)) @class.def

;; Exported interface: `export interface Foo { ... }`
(export_statement
  (interface_declaration
    name: (type_identifier) @interface.name)) @interface.def

;; Exported type alias: `export type Foo = ...`
(export_statement
  (type_alias_declaration
    name: (type_identifier) @typealias.name)) @typealias.def

;; Exported enum: `export enum Foo { ... }` (incl. const enums)
(export_statement
  (enum_declaration
    name: (identifier) @enum.name)) @enum.def

;; Exported const/let: `export const foo = ...`
(export_statement
  (lexical_declaration
    (variable_declarator
      name: (identifier) @const.name))) @const.def

;; Exported var: `export var foo = ...`
(export_statement
  (variable_declaration
    (variable_declarator
      name: (identifier) @const.name))) @const.def

;; Class methods (any class, exported or not). Class name is captured so the
;; Symbol's `parent` field can be populated.
(class_declaration
  name: (type_identifier) @method.parent
  body: (class_body
    (method_definition
      name: (property_identifier) @method.name
      body: (statement_block) @method.body) @method.def))

(abstract_class_declaration
  name: (type_identifier) @method.parent
  body: (class_body
    (method_definition
      name: (property_identifier) @method.name
      body: (statement_block) @method.body) @method.def))

;; Class fields (`public_field_definition` covers both explicit-public and
;; default-visibility fields in tree-sitter-typescript).
(class_declaration
  name: (type_identifier) @field.parent
  body: (class_body
    (public_field_definition
      name: (property_identifier) @field.name) @field.def))

(abstract_class_declaration
  name: (type_identifier) @field.parent
  body: (class_body
    (public_field_definition
      name: (property_identifier) @field.name) @field.def))
