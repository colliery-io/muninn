; C symbol extraction queries

; Functions
(function_definition
  declarator: (function_declarator
    declarator: (identifier) @name)) @function

; Function declarations (prototypes)
(declaration
  declarator: (function_declarator
    declarator: (identifier) @name)) @function_decl

; Structs
(struct_specifier
  name: (type_identifier) @name) @struct

; Enums
(enum_specifier
  name: (type_identifier) @name) @enum

; Typedefs
(type_definition
  declarator: (type_identifier) @name) @typedef

; Global variables
(declaration
  declarator: (identifier) @name) @variable
