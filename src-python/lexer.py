from consts import Consts
from error import Error
from tokens import Token


class Lexer:
    def __init__(self, source_code: str):
        self.code = source_code
        self.current = None
        self.indice, self.coluna, self.linha = 0, 0, 0

    def __advance(self):
        if self.coluna >= len(self.code):
            self.current = None
            return None

        self.current = self.code[self.coluna]

        return self.__advanceCalc(self.current)

    def __advanceCalc(self, _char: str):
        if _char in Consts.DIGITOS:
            return self.__makeNumber()
        if _char == Consts.QUOTATION_MARK:
            return self.__makeString()
        if _char in Consts.LETRAS or _char == Consts.UNDER:
            return self.__makeIdentifier()
        if _char in (
            Consts.PLUS,
            Consts.MINUS,
            Consts.MUL,
            Consts.DIV,
            Consts.POW,
            Consts.EQ,
            Consts.LPAR,
            Consts.RPAR,
            Consts.LSQUARE,
            Consts.RSQUARE,
            Consts.COMMA,
        ):
            return Token(_char)

        raise Error(f"Unknown character: {self.current}")

    def makeTokens(self):
        tokens = []
        while self.coluna < len(self.code):
            if self.code[self.coluna].isspace():
                self.coluna += 1
                self.indice += 1
                continue

            token_start = self.coluna
            new_token = self.__advance()
            if new_token is None:
                break

            tokens.append(new_token)

            if self.coluna == token_start:
                self.coluna += 1
            self.indice = self.coluna

        tokens.append(Token(Consts.EOF))
        return tokens, None

    def __makeNumber(self):
        value = ""
        dot_counter = 0
        while self.current is not None:
            if self.current.isnumeric():
                value += self.current
            elif self.current == "." and dot_counter == 0:
                dot_counter += 1
                value += self.current
            else:
                break

            if self.__nextCurrent() is False:
                break

        if dot_counter > 0:
            return Token(Consts.FLOAT, float(value))

        return Token(Consts.INT, int(value))

    def __makeIdentifier(self):
        value = ""
        while self.current is not None and (
            self.current in Consts.LETRAS_DIGITOS or self.current == Consts.UNDER
        ):
            value += self.current
            if self.__nextCurrent() is False:
                break

        if value in Consts.KEYS:
            return Token(Consts.KEY, value)
        if value == Consts.NULL:
            return Token(Consts.NULL, value)
        return Token(Consts.ID, value)

    def __makeString(self):
        value = self.current
        if self.__nextCurrent() is False:
            raise Error("Quote is never closed")

        while self.current != Consts.QUOTATION_MARK:
            value += self.current
            if self.__nextCurrent() is False:
                raise Error("Quote is never closed")

        value += self.current
        self.__nextCurrent()
        return Token(Consts.STRING, value)

    def __nextCurrent(self) -> bool:
        self.coluna += 1
        self.indice += 1
        if self.coluna >= len(self.code):
            return False
        self.current = self.code[self.coluna]

        return True
