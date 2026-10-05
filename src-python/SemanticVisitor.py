class Visitor:
    def __init__(self):
        pass

class NoNumber:
    def __init__(self, tok):
        self.tok = tok

    def __repr__(self) -> str:
        return f'{self.tok}'

class NoOpBinaria:
    def __init__(self, leftNode, opToken, rightNode) -> None:
        self.noEsq = leftNode
        self.opToken = opToken
        self.noDir = rightNode

    def __repr__(self) -> str:
        return f'{(self.noEsq), (self.opToken), (self.noDir)}'

    @staticmethod
    def Perform(Gvar1, ops, GVar2=None):
        if GVar2 == None: GVar2 = Gvar1
        ast = Gvar1.GetParserManager()
        op_bin_ou_esq = ast.registry(Gvar1.Rule())

        if ast.error:
            return ast

        while Gvar1.CurrentToken().type in ops:
            token_operator = Gvar1.CurrentToken()
            Gvar1.NextToken()
            lado_direito = ast.registry(GVar2.Rule())
            if ast.error: return ast
            op_bin_ou_esq = NoOpBinaria(op_bin_ou_esq, token_operator, lado_direito)
        return ast.success(op_bin_ou_esq)

class NoOpUnaria:
    def __init__(self, opTok, node) -> None:
        self.opTok = opTok
        self.node = node

    def __repr__(self):
        return f'{(self.opTok), (self.node)}'
