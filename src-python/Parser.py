from error import Error
from grammar import Grammar


class AstInfo:
    singleton = None

    def __init__(self):
        if AstInfo.singleton != None:
            return

        self.error = None
        self.node = None
        AstInfo.singleton = self

    def registry(self, ast_info):
        if ast_info: self.error = ast_info.error
        return ast_info

    def success(self, node):
        self.node = node
        return self

    def fail(self, errorMsg):
        if not self.error:
            self.error = Error(errorMsg)
        return self

    @staticmethod
    def resetSingletonErrorForNewParsing():
        AstInfo.singleton.error = None

    @staticmethod
    def singletonInstance():
        if AstInfo.singleton == None:
            AstInfo.singleton = AstInfo()
        return AstInfo.singleton

class Parser:
    singleton = None

    def __init__(self):
        if Parser.singleton != None:
            return

        self.__start([])
        Parser.singleton = self

    def NextToken(self):
        self.tokenIndex += 1
        if self.tokenIndex < len(self.tokens):
            self.currentToken = self.tokens[self.tokenIndex]

        return self.currentToken

    def __start(self, _tokens):
        self.tokens = _tokens
        self.tokenIndex = -1
        self.currentToken = None
        self.manager = AstInfo.singletonInstance()
        self.manager.resetSingletonErrorForNewParsing()

    def Manager(self):
        return self.manager

    def CurrentToken(self):
        return self.currentToken

    def Parsing(self, _tokens):
        self.__reset(_tokens)
        return Grammar.StartSymbol(self)

    def __reset(self, _tokens):
        self.__start(_tokens)
        self.NextToken()

    @staticmethod
    def singletonInstance():
        if Parser.singleton == None:
            Parser.singleton = Parser()
        return Parser.singleton
