; Comments
(comment) @comment

; Strings
(encapsed_string) @string
(string) @string
(heredoc) @string

; Variables
(variable_name) @variable
(php_variable) @variable

; Functions
(function_definition name: (name) @function)
(method_declaration name: (name) @method)
(function_call_expression function: (qualified_name) @function)
(method_call_expression name: (name) @method)
(scoped_call_expression name: (name) @function)

; Classes & Interfaces
(class_declaration name: (name) @type)
(interface_declaration name: (name) @type)
(trait_declaration name: (name) @type)
(enum_declaration name: (name) @type)
(anonymous_function) @function

; Namespace
(namespace_definition name: (namespace_name) @namespace)
(use_declaration) @include
(namespace_use_clause name: (namespace_name) @type)

; Keywords
[
  "abstract"
  "as"
  "break"
  "case"
  "catch"
  "class"
  "clone"
  "const"
  "continue"
  "declare"
  "default"
  "die"
  "do"
  "echo"
  "else"
  "elseif"
  "enddeclare"
  "endfor"
  "endforeach"
  "endif"
  "endswitch"
  "endwhile"
  "enum"
  "exit"
  "extends"
  "final"
  "finally"
  "fn"
  "for"
  "foreach"
  "function"
  "global"
  "goto"
  "if"
  "implements"
  "include"
  "include_once"
  "instanceof"
  "insteadof"
  "interface"
  "match"
  "named_argument"
  "namespace"
  "new"
  "private"
  "protected"
  "public"
  "readonly"
  "require"
  "require_once"
  "return"
  "static"
  "switch"
  "throw"
  "trait"
  "try"
  "use"
  "var"
  "while"
  "yield"
] @keyword

; Control flow
[
  "if"
  "else"
  "elseif"
  "for"
  "foreach"
  "while"
  "switch"
  "case"
  "default"
  "match"
  "break"
  "continue"
  "return"
  "throw"
  "try"
  "catch"
  "finally"
] @keyword.control

; Operators
[
  "and"
  "or"
  "xor"
  "not"
] @keyword.operator

; Types
[
  "int"
  "float"
  "bool"
  "string"
  "void"
  "null"
  "true"
  "false"
  "mixed"
  "never"
  "array"
  "object"
  "callable"
  "iterable"
  "self"
  "parent"
  "static"
] @type.builtin

; Numbers
(integer) @number
(float) @number

; Attributes
(attribute) @attribute

; Punctuation
"," @punctuation.delimiter
";" @punctuation.delimiter
"." @punctuation.delimiter
"->" @punctuation.delimiter
"=>" @punctuation.delimiter
"::" @punctuation.delimiter
"\\" @punctuation.delimiter

; Brackets
"(" @punctuation.bracket
")" @punctuation.bracket
"{" @punctuation.bracket
"}" @punctuation.bracket
"[" @punctuation.bracket
"]" @punctuation.bracket

; Operators
[
  "+"
  "-"
  "*"
  "/"
  "%"
  "="
  "=="
  "==="
  "!="
  "!=="
  "<"
  "<="
  ">"
  ">="
  "<=>"
  "."
  "&"
  "|"
  "^"
  "~"
  "!"
  "&&"
  "||"
  "??"
  "?"
  ":"
  "++"
  "--"
  "+="
  "-="
  "*="
  "/="
  ".="
  "%="
  "&="
  "|="
  "^="
  "<<="
  ">>="
  "**"
  "**="
  "<<"
  ">>"
  "@"
  "..."
] @operator
