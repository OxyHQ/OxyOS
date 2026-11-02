/**
 * Where an item is in its collection, and what activating it does (scenarios/accessibility-collections.json,
 * docs/adr/0039-keyboard-access.mdx): a list of three rows (`list`, `listitem`), one with a hint
 * (`accessibilityHint`), as VoiceOver says a row "3 of 21" and then its help; and rows of a longer
 * list that shows only some of them, each saying where it is among them all through Android's
 * `accessibilityCollection` and `accessibilityCollectionItem`, as a virtualized list does.
 */
import * as React from 'react';
import { Pressable, Text, View } from 'react-native';

const ROW = { paddingVertical: 8, paddingHorizontal: 12, borderRadius: 6, backgroundColor: '#eef1f5' } as const;

function Row({ id, name, hint, item }: { id: string; name: string; hint?: string; item?: { rowIndex: number } }) {
  return (
    <Pressable
      testID={id}
      focusable
      accessibilityRole="listitem"
      accessibilityLabel={name}
      accessibilityHint={hint}
      // React Native's Android-only props: where the row is in a list that shows only some rows.
      {...(item ? ({ accessibilityCollectionItem: { rowIndex: item.rowIndex, rowSpan: 1, columnIndex: 0, columnSpan: 1, heading: false } } as object) : {})}
      onPress={() => {}}
      style={ROW}
    >
      <Text>{name}</Text>
    </Pressable>
  );
}

export default function CollectionProbe() {
  return (
    <View style={{ padding: 24, gap: 16 }}>
      {/* A view that only carries accessibility props is flattened away by Fabric, as on Android:
          a list's container keeps its node with `collapsable={false}` (a scroll view always does). */}
      <View collapsable={false} accessibilityRole="list" accessibilityLabel="Documentos" style={{ gap: 4 }}>
        <Row id="notes" name="notas.txt" />
        <Row id="photo" name="foto.png" hint="Abre la foto en Vista previa" />
        <Row id="report" name="informe.pdf" />
      </View>
      <View
        collapsable={false}
        accessibilityRole="list"
        accessibilityLabel="Historial"
        {...({ accessibilityCollection: { rowCount: 21, columnCount: 1, hierarchical: false } } as object)}
        style={{ gap: 4 }}
      >
        <Row id="row-3" name="Fila 3" item={{ rowIndex: 2 }} />
        <Row id="row-4" name="Fila 4" item={{ rowIndex: 3 }} />
      </View>
    </View>
  );
}
