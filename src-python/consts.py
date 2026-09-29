import string


class Consts:
    DIGITOS = "0123456789"
    DIGITOS_ARRAY = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]
    LETRAS = string.ascii_letters
    LETRAS_DIGITOS = DIGITOS + LETRAS
    UNDER = "_"
    INT = "INT"
    FLOAT = "FLOAT"
    PLUS = "+"
    MINUS = "-"
    MUL = "*"
    DIV = "/"
    LPAR = "("
    RPAR = ")"
    EOF = "$EOF"
    EQ = "="
    POW = "^"
    ID = "ID"
    KEY = "KEY"
    NULL = "null"
    STRING = "STRING"
    LSQUARE = "["  # Left  Box brackets [
    RSQUARE = "]"  # Right Box brackets ]
    COMMA = ","
    WHITESPACE = " "
    QUOTATION_MARK = "\""

    # Exemplos de Palavras reservadas
    LET = "let"
    IF = "if"
    WHILE = "while"
    FOR = "for"
    KEYS = [LET, IF, WHILE, FOR]
