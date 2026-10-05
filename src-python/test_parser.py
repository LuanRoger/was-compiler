import unittest

from lexer import Lexer
from Parser import Parser
from SemanticVisitor import NoNumber, NoOpBinaria


class ParserTests(unittest.TestCase):
    def parse(self, source):
        tokens, error = Lexer(source).makeTokens()
        self.assertIsNone(error)
        return Parser.singletonInstance().Parsing(tokens)

    def tree(self, node):
        if isinstance(node, NoNumber):
            return node.tok.value
        self.assertIsInstance(node, NoOpBinaria)
        return (self.tree(node.noEsq), node.opToken.type, self.tree(node.noDir))

    def test_number(self):
        result = self.parse("42")
        self.assertIsNone(result.error)
        self.assertEqual(self.tree(result.node), 42)

    def test_binary_operators(self):
        for operator in ("+", "-", "*", "/"):
            with self.subTest(operator=operator):
                result = self.parse(f"8{operator}2")
                self.assertIsNone(result.error)
                self.assertEqual(self.tree(result.node), (8, operator, 2))

    def test_operator_precedence(self):
        result = self.parse("1+3*8")
        self.assertIsNone(result.error)
        self.assertEqual(self.tree(result.node), (1, "+", (3, "*", 8)))

    def test_left_associativity(self):
        result = self.parse("8-3-2")
        self.assertIsNone(result.error)
        self.assertEqual(self.tree(result.node), ((8, "-", 3), "-", 2))

    def test_parenthesized_sample(self):
        result = self.parse("1+3*8*(1+2)")
        self.assertIsNone(result.error)
        self.assertEqual(
            self.tree(result.node),
            (1, "+", ((3, "*", 8), "*", (1, "+", 2))),
        )

    def test_nested_parentheses_and_float(self):
        result = self.parse("((1.5+2)*3)")
        self.assertIsNone(result.error)
        self.assertEqual(self.tree(result.node), ((1.5, "+", 2), "*", 3))

    def test_invalid_expressions(self):
        for source in ("", "1+", "(1+2", "(1+)", "1 2"):
            with self.subTest(source=source):
                self.assertIsNotNone(self.parse(source).error)

    def test_valid_expression_after_error(self):
        self.assertIsNotNone(self.parse("1+").error)
        result = self.parse("2+3")
        self.assertIsNone(result.error)
        self.assertEqual(self.tree(result.node), (2, "+", 3))


if __name__ == "__main__":
    unittest.main()
