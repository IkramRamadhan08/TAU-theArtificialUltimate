; Indent after opening braces
("{" @open "}" @close) @indent

; Indent after opening parentheses for arguments
(arguments "(" @open ")" @close) @indent

; Indent after opening brackets
("[" @open "]" @close) @indent
