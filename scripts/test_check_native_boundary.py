"""The boundary script's own checks: `python3 scripts/test_check_native_boundary.py`.

The crate layers (docs/native-architecture.mdx, Crate layers) on metadata written by hand: a clean
graph passes, an edge up the layers, into another program or past a crate's visibility fails, and
so do an allowed edge the graph no longer has and a member without a layer."""

import importlib.util
import pathlib
import unittest

SCRIPT = pathlib.Path(__file__).with_name("check-native-boundary.py")
spec = importlib.util.spec_from_file_location("check_native_boundary", SCRIPT)
boundary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(boundary)

LAYERS = {
    "leaf": ("leaf-a",),
    "protocol": ("wire",),
    "system": ("adapter",),
    "service": ("service-a", "service-b"),
    "display": ("compositor",),
    "client": ("client",),
    "program": ("tool-a", "tool-b"),
}
VISIBLE_TO = {"compositor": {"tool-a"}}


def metadata(graph):
    """Cargo metadata for members named by `graph`'s keys, each depending on its values; a value
    `(name, kind)` is a dependency of that kind."""
    packages = []
    for name, dependencies in graph.items():
        listed = []
        for dependency in dependencies:
            target, kind = dependency if isinstance(dependency, tuple) else (dependency, None)
            listed.append({"name": target, "kind": kind})
        packages.append({"name": name, "id": f"{name} 0.1.0", "dependencies": listed})
    # A third-party package: never a member, never checked for layers.
    packages.append({"name": "serde", "id": "serde 1.0.0", "dependencies": []})
    return {"packages": packages, "workspace_members": [f"{name} 0.1.0" for name in graph]}


CLEAN = {
    "leaf-a": [],
    "wire": ["adapter"],
    "adapter": ["leaf-a", "serde"],
    "service-a": ["adapter", "service-b"],
    "service-b": ["leaf-a"],
    "compositor": ["wire", "service-a"],
    "client": ["wire", "adapter"],
    "tool-a": ["compositor", "client"],
    "tool-b": ["service-a"],
}


def check(graph, allowed=None):
    return boundary.check_layers(
        metadata(graph), layers=LAYERS, may_use=boundary.MAY_USE, visible_to=VISIBLE_TO,
        allowed=allowed or {},
    )


def with_edge(source, target):
    graph = {name: list(dependencies) for name, dependencies in CLEAN.items()}
    graph[source].append(target)
    return graph


class Layers(unittest.TestCase):
    def test_a_clean_graph_passes(self):
        self.assertEqual(check(CLEAN), [])

    def test_an_edge_up_the_layers_fails(self):
        violations = check(with_edge("adapter", "service-b"))
        self.assertEqual(len(violations), 1)
        self.assertIn("adapter (system) -> service-b (service)", violations[0])

    def test_a_service_speaking_the_display_protocol_fails(self):
        self.assertEqual(len(check(with_edge("service-b", "wire"))), 1)

    def test_a_client_linking_a_service_fails(self):
        self.assertEqual(len(check(with_edge("client", "service-a"))), 1)

    def test_a_program_using_another_program_fails(self):
        violations = check(with_edge("tool-b", "tool-a"))
        self.assertEqual(len(violations), 1)
        self.assertIn("may not use the program layer", violations[0])

    def test_a_crate_past_its_visibility_fails(self):
        violations = check(with_edge("tool-b", "compositor"))
        self.assertEqual(len(violations), 1)
        self.assertIn("only tool-a may use it", violations[0])

    def test_a_dev_dependency_may_reach_anything(self):
        self.assertEqual(check(with_edge("leaf-a", ("tool-a", "dev"))), [])

    def test_a_build_dependency_counts(self):
        self.assertEqual(len(check(with_edge("leaf-a", ("adapter", "build")))), 1)

    def test_an_allowed_edge_passes(self):
        graph = with_edge("adapter", "service-b")
        self.assertEqual(check(graph, allowed={("adapter", "service-b"): "a reason"}), [])

    def test_an_allowed_edge_the_graph_no_longer_has_fails(self):
        violations = check(CLEAN, allowed={("adapter", "service-b"): "a reason"})
        self.assertEqual(len(violations), 1)
        self.assertIn("no longer exists", violations[0])

    def test_a_member_without_a_layer_fails(self):
        graph = dict(CLEAN, newcomer=["leaf-a"])
        violations = check(graph)
        self.assertEqual(len(violations), 1)
        self.assertIn("without a layer: newcomer", violations[0])

    def test_a_layered_crate_the_workspace_lacks_fails(self):
        graph = {name: deps for name, deps in CLEAN.items() if name != "tool-b"}
        violations = check(graph)
        self.assertEqual(len(violations), 1)
        self.assertIn("lacks: tool-b", violations[0])


class Table(unittest.TestCase):
    def test_every_layer_has_its_rule(self):
        self.assertEqual(set(boundary.LAYERS), set(boundary.MAY_USE))

    def test_no_layer_may_use_programs(self):
        for layer, uses in boundary.MAY_USE.items():
            self.assertNotIn("program", uses, layer)

    def test_every_allowed_edge_says_why(self):
        for edge, reason in boundary.ALLOWED.items():
            self.assertTrue(reason.strip(), edge)


if __name__ == "__main__":
    unittest.main()
