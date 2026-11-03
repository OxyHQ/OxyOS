/**
 * Tabs from the keyboard (scenarios/tabs-keys.json, docs/adr/0039-keyboard-access.mdx): Bloom's
 * `Tabs` and `CategoryBar`, each one Tab stop on its selected tab, as a Mac's tab view and
 * segmented tabs and WAI-ARIA's tabs pattern: ← → Home End select the tab they reach (automatic
 * activation), the keyboard following once the value lands, so each tab is said once, already
 * selected, with its place among the list's tabs ("General, seleccionado, pestaña, 1 de 3").
 */
import * as React from 'react';
import { View } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Button } from '@oxy.so/bloom/button';
import { CategoryBar } from '@oxy.so/bloom/category-bar';
import { RiHomeLine } from '@oxy.so/bloom/icons/RiHomeLine';
import { RiSearchLine } from '@oxy.so/bloom/icons/RiSearchLine';
import { RiUserLine } from '@oxy.so/bloom/icons/RiUserLine';
import { Tabs, TabsTrigger } from '@oxy.so/bloom/tabs';
import { Text } from '@oxy.so/bloom/typography';
import { oxyosSurfaces } from './oxyos/platform-surface';

const SECTIONS = [
  { value: 'general', label: 'General' },
  { value: 'advanced', label: 'Avanzado' },
  { value: 'about', label: 'Acerca de' },
];

const CATEGORIES = [
  { key: 'homes', label: 'Casas', icon: RiHomeLine },
  { key: 'search', label: 'Buscar', icon: RiSearchLine },
  { key: 'people', label: 'Personas', icon: RiUserLine },
];

export default function TabsProbe() {
  const [section, setSection] = React.useState('general');
  const [category, setCategory] = React.useState('homes');
  return (
    <SafeAreaProvider>
      <BloomProvider platform={oxyosSurfaces} locale="es">
        <View style={{ padding: 24, gap: 16, alignItems: 'flex-start' }}>
          <Tabs label="Secciones" value={section} onValueChange={setSection} testID="sections">
            {SECTIONS.map((tab) => (
              <TabsTrigger key={tab.value} value={tab.value} label={tab.label} testID={`tab-${tab.value}`} />
            ))}
          </Tabs>
          <Text testID="section">{`Sección: ${SECTIONS.find((tab) => tab.value === section)?.label}`}</Text>
          <CategoryBar
            accessibilityLabel="Categorías"
            items={CATEGORIES}
            value={category}
            onValueChange={setCategory}
            style={{ alignSelf: 'stretch' }}
            testID="categories"
          />
          <Button testID="done">Listo</Button>
        </View>
      </BloomProvider>
    </SafeAreaProvider>
  );
}
