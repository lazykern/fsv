import json
import unittest

import gen_tree


class GeneratorTests(unittest.TestCase):
    def test_integer_parsers(self):
        self.assertEqual(gen_tree.int_parser({"type": "integer"}), "crate::pyint::INT")
        self.assertEqual(
            gen_tree.int_parser({"type": "integer range", "min": 1, "max": None}),
            "crate::pyint::POSITIVE",
        )
        self.assertEqual(
            gen_tree.int_parser({"type": "integer range", "min": 1, "max": 100}),
            "crate::pyint::PER_PAGE",
        )
        with self.assertRaises(SystemExit):
            gen_tree.int_parser({"name": "x", "type": "integer range", "min": 2})

    def test_billable_aliases(self):
        fixture = gen_tree.CRATE / "tests" / "fixtures" / "cli_tree.json"
        nodes = gen_tree.walk(json.loads(fixture.read_text()), [])
        for path in ("fsv changes time write", "fsv problems time write"):
            with self.subTest(path=path):
                node = next(n for n in nodes if n["path"] == path)
                attrs = next(a for a, name, _ in gen_tree.fields(node["params"]) if name == "non_billable")
                self.assertIn('alias = "no-billable"', attrs)

    def test_generated_tree_is_current(self):
        self.assertEqual(gen_tree.OUT.read_text(), gen_tree.generate())


if __name__ == "__main__":
    unittest.main()
