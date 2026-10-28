// Builds a two-panel tree with text and a scroll view through Fabric's JSI binding.
var UI = nativeFabricUIManager;
var next = 2;
function node(rootTag, name, props, children) {
  var tag = next; next += 2;
  var n = UI.createNode(tag, name, rootTag, props, {});
  (children || []).forEach(function (c) { UI.appendChild(n, c); });
  return n;
}
globalThis.RN$AppRegistry = {
  runApplication: function (appKey, params) {
    print('runApplication ' + appKey);
    var root = params.rootTag;
    var text = node(root, 'Paragraph', {}, [node(root, 'RawText', { text: 'Hello from Fabric on Linux' })]);
    var sidebar = node(root, 'View', { width: 240, padding: 16, backgroundColor: 0xff202833 }, [text]);
    var rows = [];
    for (var i = 0; i < 20; i++) rows.push(node(root, 'View', { height: 48, marginBottom: 1, backgroundColor: 0xffe8e8ee }));
    var content = node(root, 'View', { backgroundColor: 0xffffffff }, rows);
    var scroll = node(root, 'RCTScrollView', { flex: 1 }, [content]);
    var shell = node(root, 'View', { flex: 1, flexDirection: 'row', backgroundColor: 0xff101418 }, [sidebar, scroll]);
    var set = UI.createChildSet(root);
    UI.appendChildToSet(set, shell);
    UI.completeRoot(root, set);
    print('completeRoot for surface ' + root);
  },
};
globalThis.RN$stopSurface = function (rootTag) { print('stop ' + rootTag); };
