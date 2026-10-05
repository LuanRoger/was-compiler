from error import Error
from consts import Consts
from SemanticVisitor import NoNumber, NoOpBinaria

class Grammar:
    def __init__(self, parser) -> None:
        self.parser = parser

    def Rule(self):
        return self.GetParserManager().fail(f"{Error.parserError}, Implementar sua regra de produção")

    def CurrentToken(self):
        return self.parser.CurrentToken()

    def NextToken(self):
        return self.parser.NextToken()

    def GetParserManager(self):
        return self.parser.Manager()

    @staticmethod
    def StartSymbol(parser):
        resultado = Exp(parser).Rule()
        if parser.CurrentToken().type != Consts.EOF:
            return resultado.fail("Error de parse")

        return resultado

class Exp(Grammar):
    def Rule(self):
        pr = self.GetParserManager()
        node = pr.registry(NoOpBinaria.Perform(Term(self.parser), (Consts.PLUS, Consts.MINUS)))

class Term(Grammar):
    def Rule(self):
        return NoOpBinaria.Perform(Factor(self.parser), [Consts.MUL, Consts.DIV])

class Factor(Grammar):
    def Rule(self):
        return Atom(self.parser).Rule()

class Atom(Grammar):
    def Rule(self):
        ast = self.GetParserManager()
        tok = self.CurrentToken()
        if tok.type in (Consts.INT, Consts.FLOAT):
            self.NextToken()
            return ast.success(NoNumber(tok))

        elif tok.type == Consts.LPAR:
            self.NextToken()
            exp = ast.registry(Exp(self.parser).Rule())
            if self.CurrentToken().type == Consts.RPAR:
                self.NextToken()
                return ast.success(exp)
            return ast.fail("Esperando o RPAR")

        return ast.fail("Error")
