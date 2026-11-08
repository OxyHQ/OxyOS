// docs/platform-api.mdx, generated from @oxyos/platform's own types and their documentation
// (docs/adr/0058-the-platform-api.mdx, decision 9), so the page cannot drift from the package:
// `node platform-docs.mjs` writes it, `node platform-docs.mjs --check` fails where it differs (the
// shell-preview tests run that check). The compiler reads the package as this directory's type check
// does (tsconfig.json), and the page says, for every framework the package exports, its
// documentation and each of its members' signature and documentation, one level of namespaces
// deep (`files.tags`, `windows.menus`).
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

const here = path.dirname(fileURLToPath(import.meta.url));
const INDEX = path.resolve(here, '../rn-host-spike/sample/oxyos/platform/src/index.ts');
const PAGE = path.resolve(here, '../../docs/platform-api.mdx');

/** The package as the type check reads it. */
function program() {
  const config = ts.readConfigFile(path.join(here, 'tsconfig.json'), ts.sys.readFile);
  const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, here);
  return ts.createProgram([INDEX], parsed.options);
}

const oneLine = (text) => text.replace(/\s+/g, ' ').trim();
const documentation = (symbol, checker) => oneLine(ts.displayPartsToString(symbol.getDocumentationComment(checker)));
/** A type as written for a reader: without the compiler's `import("./file.ts").` qualifiers. */
const written = (text) => oneLine(text.replace(/import\("[^"]*"\)\./g, ''));
/** Whether the symbol is the runtime's own (`@internal`), never a contract (decision 2). */
const internal = (symbol) => symbol.getJsDocTags().some((tag) => tag.name === 'internal');
/** What every framework's `available` says (decision 3, "Present or not"). */
const AVAILABLE = 'Whether this host has it: where it does not, every call rejects with `unavailable`.';

/**
 * A framework's documentation: its export's, or, for one destructured from an object
 * (`export const { network } = frameworks`), the documentation of the variable that object took it
 * from.
 */
function frameworkDocumentation(symbol, checker) {
  const own = documentation(symbol, checker);
  if (own) return own;
  const binding = symbol.valueDeclaration;
  if (!binding || !ts.isBindingElement(binding)) return '';
  const pattern = binding.parent;
  const declaration = pattern.parent;
  if (!ts.isVariableDeclaration(declaration) || !declaration.initializer) return '';
  const from = checker.getTypeAtLocation(declaration.initializer).getProperty(symbol.getName());
  const shorthand = from?.valueDeclaration;
  if (!shorthand || !ts.isShorthandPropertyAssignment(shorthand)) return from ? documentation(from, checker) : '';
  const value = checker.getShorthandAssignmentValueSymbol(shorthand);
  return value ? documentation(value, checker) : '';
}
/** A Markdown table cell: pipes escaped. */
const cell = (text) => text.replace(/\|/g, '\\|');

/** The members of `type` worth documenting: its own properties, sorted as declared. */
function members(type, checker, at) {
  return checker
    .getPropertiesOfType(type)
    .filter((member) => !member.getName().startsWith('_'))
    .map((member) => ({ member, type: checker.getTypeOfSymbolAtLocation(member, at) }));
}

/** One member's row: its name and signature (or type), and what its documentation says. */
function row(prefix, member, type, checker, at) {
  const name = `${prefix}${member.getName()}`;
  const signatures = type.getCallSignatures();
  const shape = signatures.length
    ? signatures
        .map((signature) => checker.signatureToString(signature, at, ts.TypeFormatFlags.NoTruncation))
        .join(' / ')
    : checker.typeToString(type, at, ts.TypeFormatFlags.NoTruncation);
  const doc = documentation(member, checker) || (member.getName() === 'available' ? AVAILABLE : '');
  return `| \`${name}\` | \`${cell(written(shape))}\` | ${cell(doc)} |`;
}

/** Whether `type` is a namespace of the framework's (an object of functions), not a value. */
const isNamespace = (type, checker, at) =>
  !type.getCallSignatures().length &&
  (type.flags & ts.TypeFlags.Object) !== 0 &&
  !written(checker.typeToString(type, at)).startsWith('Followed<') &&
  members(type, checker, at).some(({ type: inner }) => inner.getCallSignatures().length > 0);

export function render() {
  const built = program();
  const checker = built.getTypeChecker();
  const source = built.getSourceFile(INDEX);
  const module = checker.getSymbolAtLocation(source);
  const exported = checker.getExportsOfModule(module);
  const frameworks = exported
    .filter((symbol) => symbol.flags & ts.SymbolFlags.Variable && !internal(symbol))
    .map((symbol) => ({ symbol, type: checker.getTypeOfSymbolAtLocation(symbol, source) }))
    // A framework is an object with an `available` (decision 3, "Present or not").
    .filter(({ type }) => type.getProperty('available'))
    .sort((a, b) => a.symbol.getName().localeCompare(b.symbol.getName()));

  const lines = [
    '---',
    'title: The platform API',
    'description: "@oxyos/platform, the API through which everything that runs on OxyOS reaches the platform: every framework, its calls and followed state, generated from the package."',
    'order: 8',
    '---',
    '',
    '{/* Generated by tools/shell-preview/platform-docs.mjs from @oxyos/platform\'s types: do not edit. */}',
    '',
    '# The platform API',
    '',
    "`@oxyos/platform` is OxyOS's platform API ([ADR 0058](./adr/0058-the-platform-api)), in the runtime's framework. Its frameworks each own one domain; every call that reaches another process or the disk returns a promise, takes `{signal?: AbortSignal}` and fails by rejecting with a `PlatformError` (`domain`, the framework, and `code`: `notPermitted`, `needsInput`, `unavailable`, `notFound`, `exists`, `invalid`, `busy`, `locked`, `cancelled` or `failed`, with the service's own word as `reason`); what changes on its own is a `Followed<T>` (`current()`, `subscribe(listener)`), which `useFollowed` from `@oxyos/platform/react` gives a view. Each framework's `available` says whether this host has it.",
    '',
    'This page is generated from the package\'s types and their documentation (`tools/shell-preview/platform-docs.mjs`); the shell preview\'s tests fail where it differs from them.',
    '',
  ];
  for (const { symbol, type } of frameworks) {
    const name = symbol.getName();
    lines.push(`## \`${name}\``, '');
    const about = frameworkDocumentation(symbol, checker);
    if (about) lines.push(about, '');
    lines.push('| Member | Signature | What it does |', '| --- | --- | --- |');
    const namespaces = [];
    for (const { member, type: memberType } of members(type, checker, source)) {
      if (isNamespace(memberType, checker, source)) {
        namespaces.push({ member, memberType });
        continue;
      }
      lines.push(row(`${name}.`, member, memberType, checker, source));
    }
    for (const { member, memberType } of namespaces) {
      for (const inner of members(memberType, checker, source)) {
        lines.push(row(`${name}.${member.getName()}.`, inner.member, inner.type, checker, source));
      }
    }
    lines.push('');
  }
  return `${lines.join('\n').trimEnd()}\n`;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const page = render();
  if (process.argv.includes('--check')) {
    const written = fs.existsSync(PAGE) ? fs.readFileSync(PAGE, 'utf8') : '';
    if (written !== page) {
      console.error('docs/platform-api.mdx differs from @oxyos/platform: run `node platform-docs.mjs`');
      process.exit(1);
    }
  } else {
    fs.writeFileSync(PAGE, page);
  }
}
