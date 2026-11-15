import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { expect, it } from "vitest";

it("keeps floating Ariakit primitives behind the shared overlay boundary", () => {
  const directory = dirname(fileURLToPath(import.meta.url));
  const sourceRoot = join(directory, "../../..");
  const floating = new Set(["Dialog", "Menu", "Popover", "SelectPopover", "Tooltip", "Portal"]);
  const violations: string[] = [];
  function visit(folder: string) {
    for (const entry of readdirSync(folder, {withFileTypes: true})) {
      const file = join(folder, entry.name);
      if (entry.isDirectory()) { if (file !== directory) visit(file); continue; }
      if (!/\.tsx?$/.test(file) || /\.test\./.test(file)) continue;
      const source = ts.createSourceFile(file, readFileSync(file, "utf8"), ts.ScriptTarget.Latest, true);
      for (const node of source.statements) {
        if (!ts.isImportDeclaration(node) || !ts.isStringLiteral(node.moduleSpecifier) || node.moduleSpecifier.text !== "@ariakit/react") continue;
        const bindings = node.importClause?.namedBindings;
        if (!bindings || !ts.isNamedImports(bindings)) continue;
        for (const binding of bindings.elements) {
          if (floating.has((binding.propertyName ?? binding.name).text)) violations.push(relative(sourceRoot, file));
        }
      }
    }
  }
  visit(sourceRoot);
  expect(violations).toEqual([]);
});

it("keeps raw modal backdrops and non-native body portals out of features", () => {
  const directory = dirname(fileURLToPath(import.meta.url));
  const sourceRoot = join(directory, "../../..");
  const violations: string[] = [];
  function visitFolder(folder: string) {
    for (const entry of readdirSync(folder, {withFileTypes: true})) {
      const file = join(folder, entry.name);
      if (entry.isDirectory()) { if (file !== directory) visitFolder(file); continue; }
      if (!/\.tsx?$/.test(file) || /\.test\./.test(file)) continue;
      const source = ts.createSourceFile(file, readFileSync(file, "utf8"), ts.ScriptTarget.Latest, true);
      const report = (node: ts.Node, reason: string) => violations.push(`${relative(sourceRoot, file)}:${source.getLineAndCharacterOfPosition(node.getStart()).line + 1} ${reason}`);
      function visit(node: ts.Node) {
        if (ts.isCallExpression(node) && node.expression.getText(source).endsWith("createPortal") &&
          node.arguments[1]?.getText(source) === "document.body") {
          let content = node.arguments[0]!;
          while (ts.isParenthesizedExpression(content)) content = content.expression;
          const opening = ts.isJsxElement(content) ? content.openingElement : ts.isJsxSelfClosingElement(content) ? content : undefined;
          // Native showModal() owns the browser top layer. Other floating body
          // portals must declare ownership through the shared adapter.
          if (!opening || opening.tagName.getText(source) !== "dialog" || opening.attributes.properties.some(attr =>
            ts.isJsxAttribute(attr) && attr.name.getText(source) === "open")) report(node, "unowned body portal");
        }
        if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) {
          const tag = node.tagName.getText(source);
          if (/^[a-z]/.test(tag) && tag !== "dialog" && node.attributes.properties.some(attr =>
            ts.isJsxAttribute(attr) && attr.name.getText(source) === "aria-modal" &&
            attr.initializer && ts.isStringLiteral(attr.initializer) && attr.initializer.text === "true")) report(node, "raw modal backdrop");
          if (tag === "Toaster") {
            let parent: ts.Node | undefined = node.parent;
            while (parent && !(ts.isJsxElement(parent) && parent.openingElement.tagName.getText(source) === "OverlayHost")) parent = parent.parent;
            if (!parent) report(node, "third-party notification without OverlayHost");
          }
        }
        ts.forEachChild(node, visit);
      }
      visit(source);
    }
  }
  visitFolder(sourceRoot);
  expect(violations).toEqual([]);
});
