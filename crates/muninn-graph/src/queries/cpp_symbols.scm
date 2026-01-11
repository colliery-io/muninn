; C++ symbol extraction queries

; Classes
(class_specifier
  name: (type_identifier) @name) @class

; Structs
(struct_specifier
  name: (type_identifier) @name) @struct

; Functions
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @name)) @function

; Function declarations
(declaration
  declarator: (function_declarator
    declarator: (identifier) @name)) @function_decl

; Methods (inside class)
(function_definition
  declarator: (function_declarator
    declarator: (field_identifier) @name)) @method

; Namespaces
(namespace_definition
  name: (identifier) @name) @namespace

; Enums
(enum_specifier
  name: (type_identifier) @name) @enum

; Type aliases
(type_definition
  declarator: (type_identifier) @name) @typedef

; Templates
(template_declaration) @template
