/**
 * Bloom's components in their main states, each in a cell of its own at a fixed place, for
 * fidelity.py: the same gallery drawn by Chromium (react-native-web, fidelity/web.mjs) and by the
 * OxyOS host (scenarios/fidelity*.json), cropped cell by cell and compared.
 *
 * Every cell's place and size is a multiple of 4 logical pixels, so it is whole at scale 1.25 too.
 * Its content is laid out from its top left corner (or across it, `stretch`), and both sides report
 * where each cell and its content were laid out: in a browser `window.__fidelity`, on the host a
 * `[fidelity] {...}` line in the log. A content laid out at another size on the two sides is a
 * measurement difference, not a painting one.
 *
 * The page is a prop (`page`, the web build's `?page=N`) or the module's name (FidelityGallery is
 * page 1, FidelityGalleryN page N: sample/index.js). The scheme is the system's (Bloom's `system`
 * mode: the browser's prefers-color-scheme, the session's appearance); motion is reduced on both
 * sides, so nothing is moving when the frame is shot.
 */
import * as React from 'react';
import { Platform, View } from 'react-native';
import { SafeAreaProvider } from 'react-native-safe-area-context';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { useTheme } from '@oxy.so/bloom/theme';
import { PortalOutlet, PortalProvider } from '@oxy.so/bloom/portal';
import { Surface } from '@oxy.so/bloom/surface';
import {
  Button,
  CloseButton,
  GlyphButton,
  IconButton,
  InverseButton,
  LinkButton,
} from '@oxy.so/bloom/button';
import { ButtonGroup, ButtonGroupItem } from '@oxy.so/bloom/button-group';
import { TextField, TextFieldIcon, TextFieldInput, TextFieldLabel } from '@oxy.so/bloom/text-field';
import { Textarea } from '@oxy.so/bloom/textarea';
import { Select, SelectContent, SelectIcon, SelectItem, SelectItemIndicator, SelectItemText, SelectTrigger, SelectValue } from '@oxy.so/bloom/select';
import { Slider } from '@oxy.so/bloom/slider';
import { Switch } from '@oxy.so/bloom/switch';
import { Checkbox } from '@oxy.so/bloom/checkbox';
import { Radio, RadioGroup } from '@oxy.so/bloom/radio';
import { Tabs, TabsTrigger } from '@oxy.so/bloom/tabs';
import { SegmentedControl, SegmentedControlItem, SegmentedControlItemText } from '@oxy.so/bloom/segmented-control';
import { Card, CardBody, CardDescription, CardHeader, CardTitle } from '@oxy.so/bloom/card';
import { AppShell, AppShellPanel, AppShellPanels } from '@oxy.so/bloom/app-shell';
import { GroupedSection, GroupedSectionItem } from '@oxy.so/bloom/grouped-section';
import { SettingsListGroup, SettingsListItem } from '@oxy.so/bloom/settings-list';
import { Item } from '@oxy.so/bloom/item';
import { Table, TableBody, TableCell, TableColumn, TableHeader, TableRow } from '@oxy.so/bloom/table';
import { Avatar } from '@oxy.so/bloom/avatar';
import { Badge } from '@oxy.so/bloom/badge';
import { Chip } from '@oxy.so/bloom/chip';
import { Kbd } from '@oxy.so/bloom/kbd';
import { Divider } from '@oxy.so/bloom/divider';
import { Text } from '@oxy.so/bloom/typography';
import { Sidebar, SidebarItem } from '@oxy.so/bloom/sidebar';
import { Toolbar } from '@oxy.so/bloom/layout';
import { Dialog } from '@oxy.so/bloom/dialog';
import { Popover, PopoverContent, PopoverTrigger } from '@oxy.so/bloom/popover';
import { Tooltip, TooltipTextBubble, TooltipTrigger } from '@oxy.so/bloom/tooltip';
import { toast, ToastOutlet } from '@oxy.so/bloom/toast';
import { RiAddLine } from '@oxy.so/bloom/icons/RiAddLine';
import { RiHomeLine } from '@oxy.so/bloom/icons/RiHomeLine';
import { RiSearchLine } from '@oxy.so/bloom/icons/RiSearchLine';
import { RiSettings3Line } from '@oxy.so/bloom/icons/RiSettings3Line';
import { RiUserLine } from '@oxy.so/bloom/icons/RiUserLine';
import { RiHeart3Line } from '@oxy.so/bloom/icons/RiHeart3Line';
import { RiNotification3Line } from '@oxy.so/bloom/icons/RiNotification3Line';
import { RiMailLine } from '@oxy.so/bloom/icons/RiMailLine';
import { RiStarLine } from '@oxy.so/bloom/icons/RiStarLine';
import { RiMoreFill } from '@oxy.so/bloom/icons/RiMoreFill';
import { fidelityPlatform } from './FidelityPlatform';
import PAGES from './fidelity-pages.json';

type Cell = {
  id: string;
  w: number;
  h: number;
  /** Laid out across the cell (fields, cards) rather than at its natural size. */
  stretch?: boolean;
  /** Inset of the content from the cell's corner (room for a shadow). */
  inset?: number;
  /** The content fills the cell (a page that is one cell: the app shell). */
  fill?: boolean;
  render: () => React.ReactNode;
};

const noop = () => {};
const SIZES = ['xs', 'sm', 'md', 'lg'] as const;
const APPEARANCES = ['solid', 'subtle', 'outline', 'plain'] as const;
const FRUITS = [
  { value: 'apple', label: 'Apple' },
  { value: 'pear', label: 'Pear' },
];

function SelectCell({ value }: { value?: string }) {
  return (
    <Select value={value} onValueChange={noop}>
      <SelectTrigger label="Fruit">
        <SelectValue placeholder="Pick a fruit" />
        <SelectIcon />
      </SelectTrigger>
      <SelectContent
        label="Fruit"
        items={FRUITS}
        renderItem={(item: { value: string; label: string }) => (
          <SelectItem value={item.value} label={item.label}>
            <SelectItemIndicator />
            <SelectItemText>{item.label}</SelectItemText>
          </SelectItem>
        )}
      />
    </Select>
  );
}

const SIDEBAR_ITEMS = [
  { key: 'home', label: 'Home', icon: RiHomeLine },
  { key: 'search', label: 'Search', icon: RiSearchLine },
  { key: 'mail', label: 'Mail', icon: RiMailLine },
  { key: 'settings', label: 'Settings', icon: RiSettings3Line },
];

// The type scale: every family, regular and semibold, and the other weights of the body sizes.
const LARGE_FAMILIES = ['large-title', 'display-1', 'display-2', 'display-3', 'display-4', 'title-1'] as const;
const SMALL_FAMILIES = ['title-2', 'title-3', 'headline', 'body', 'body-2', 'caption-1', 'caption-2'] as const;
const FAMILY_SIZE: Record<string, number> = {
  'large-title': 64, 'display-1': 56, 'display-2': 48, 'display-3': 40, 'display-4': 32, 'title-1': 24,
  'title-2': 20, 'title-3': 18, headline: 16, body: 14, 'body-2': 13, 'caption-1': 12, 'caption-2': 11,
};
const round4 = (value: number) => Math.ceil(value / 4) * 4;
const PARAGRAPH =
  'Bloom draws the same interface on every platform. This paragraph wraps over several lines, ' +
  'where Android breaks lines for balance and a browser breaks them greedily, so its breaks may ' +
  'legitimately differ.';

function typeCells(): Cell[] {
  const cells: Cell[] = [];
  for (const family of LARGE_FAMILIES) {
    for (const weight of ['regular', 'semibold']) {
      cells.push({ id: `type.${family}.${weight}`, w: 580, h: round4(FAMILY_SIZE[family] * 1.2 + 20),
        render: () => <Text variant={`${family}-${weight}` as never} numberOfLines={1}>Bloom Aa 0123</Text> });
    }
  }
  for (const family of SMALL_FAMILIES) {
    for (const weight of ['regular', 'semibold']) {
      cells.push({ id: `type.${family}.${weight}`, w: 284, h: 48,
        render: () => <Text variant={`${family}-${weight}` as never} numberOfLines={1}>The quick brown fox 0123</Text> });
    }
  }
  for (const family of ['headline', 'body']) {
    for (const weight of ['medium', 'bold']) {
      cells.push({ id: `type.${family}.${weight}`, w: 284, h: 48,
        render: () => <Text variant={`${family}-${weight}` as never} numberOfLines={1}>The quick brown fox 0123</Text> });
    }
  }
  return cells;
}

const PAGE_CELLS: Record<number, Cell[]> = {
  1: [
    ...APPEARANCES.flatMap((appearance) =>
      SIZES.map((size) => ({
        id: `button.${appearance}.${size}`, w: 136, h: 72,
        render: () => <Button appearance={appearance} size={size}>Continue</Button>,
      }))),
    ...(['neutral', 'danger', 'success', 'warning'] as const).map((tone) => ({
      id: `button.solid.${tone}`, w: 136, h: 72,
      render: () => <Button tone={tone} appearance="solid">{tone === 'danger' ? 'Delete' : 'Continue'}</Button>,
    })),
    { id: 'button.outline.neutral', w: 136, h: 72, render: () => <Button tone="neutral" appearance="outline">Secondary</Button> },
    { id: 'button.inverse', w: 136, h: 72, render: () => <InverseButton>Inverse</InverseButton> },
    { id: 'button.disabled.solid', w: 136, h: 72, render: () => <Button disabled>Disabled</Button> },
    { id: 'button.disabled.outline', w: 136, h: 72, render: () => <Button disabled tone="neutral" appearance="outline">Disabled</Button> },
    { id: 'button.leading-icon', w: 136, h: 72, render: () => <Button leadingIcon={RiAddLine}>New</Button> },
    { id: 'button.icon-only', w: 136, h: 72, render: () => <IconButton leadingIcon={RiMoreFill} accessibilityLabel="More" /> },
    { id: 'button.close', w: 136, h: 72, render: () => <CloseButton size="md" accessibilityLabel="Close" /> },
    { id: 'button.glyph', w: 136, h: 72, render: () => <GlyphButton size={40} icon={RiMoreFill} accessibilityLabel="More" /> },
    { id: 'button.link', w: 136, h: 72, render: () => <LinkButton>Learn more</LinkButton> },
    {
      id: 'button-group', w: 280, h: 72,
      render: () => (
        <ButtonGroup accessibilityLabel="Date range">
          <ButtonGroupItem checked>Day</ButtonGroupItem>
          <ButtonGroupItem>Week</ButtonGroupItem>
          <ButtonGroupItem disabled>Month</ButtonGroupItem>
        </ButtonGroup>
      ),
    },
    {
      id: 'text-field.placeholder', w: 280, h: 104, stretch: true,
      render: () => (
        <View style={{ gap: 6 }}>
          <TextFieldLabel>Email</TextFieldLabel>
          <TextField><TextFieldInput label="Email" placeholder="you@oxy.so" onValueChange={noop} /></TextField>
        </View>
      ),
    },
    {
      id: 'text-field.filled', w: 280, h: 104, stretch: true,
      render: () => (
        <View style={{ gap: 6 }}>
          <TextFieldLabel>Name</TextFieldLabel>
          <TextField><TextFieldInput label="Name" value="Ada Lovelace" onValueChange={noop} /></TextField>
        </View>
      ),
    },
    {
      id: 'text-field.icon', w: 280, h: 104, stretch: true,
      render: () => (
        <View style={{ gap: 6 }}>
          <TextFieldLabel>Search</TextFieldLabel>
          <TextField>
            <TextFieldIcon icon={RiSearchLine} position="leading" />
            <TextFieldInput label="Search" placeholder="Search mail" onValueChange={noop} />
          </TextField>
        </View>
      ),
    },
    {
      id: 'text-field.invalid', w: 280, h: 104, stretch: true,
      render: () => (
        <View style={{ gap: 6 }}>
          <TextFieldLabel>Email</TextFieldLabel>
          <TextField invalid><TextFieldInput label="Email" value="not-an-email" invalid onValueChange={noop} /></TextField>
        </View>
      ),
    },
    {
      id: 'text-field.disabled', w: 280, h: 104, stretch: true,
      render: () => (
        <View style={{ gap: 6 }}>
          <TextFieldLabel>Team</TextFieldLabel>
          <TextField disabled><TextFieldInput label="Team" value="Design" disabled onValueChange={noop} /></TextField>
        </View>
      ),
    },
    {
      id: 'textarea.placeholder', w: 280, h: 160, stretch: true,
      render: () => <Textarea label="Notes" placeholder="Write a note" rows={3} onChangeText={noop} />,
    },
    { id: 'select.placeholder', w: 280, h: 80, stretch: true, render: () => <SelectCell /> },
    { id: 'select.value', w: 280, h: 80, stretch: true, render: () => <SelectCell value="apple" /> },
    { id: 'slider', w: 280, h: 108, render: () => <View style={{ width: 240 }}><Slider label="Volume" value={40} onValueChange={noop} /></View> },
  ],
  2: [
    { id: 'switch.on', w: 136, h: 64, render: () => <Switch checked onCheckedChange={noop} accessibilityLabel="On" /> },
    { id: 'switch.off', w: 136, h: 64, render: () => <Switch checked={false} onCheckedChange={noop} accessibilityLabel="Off" /> },
    { id: 'switch.disabled', w: 136, h: 64, render: () => <Switch checked disabled onCheckedChange={noop} accessibilityLabel="Disabled" /> },
    { id: 'switch.sm', w: 136, h: 64, render: () => <Switch checked size="sm" onCheckedChange={noop} accessibilityLabel="Small" /> },
    { id: 'checkbox.on', w: 200, h: 64, stretch: true, render: () => <Checkbox checked label="Remember me" onCheckedChange={noop} /> },
    { id: 'checkbox.off', w: 200, h: 64, stretch: true, render: () => <Checkbox checked={false} label="Remember me" onCheckedChange={noop} /> },
    { id: 'checkbox.indeterminate', w: 200, h: 64, stretch: true, render: () => <Checkbox indeterminate label="Some selected" onCheckedChange={noop} /> },
    // Laid out at its natural size, its label a `flex: 1` child of a row: Yoga gives it no width
    // (no free space to grow into), a browser its max-content width. A measurement probe.
    { id: 'checkbox.shrink-wrapped', w: 200, h: 64, render: () => <Checkbox checked label="Remember me" onCheckedChange={noop} /> },
    { id: 'radio.on', w: 200, h: 64, stretch: true, render: () => <Radio value="a" checked label="Everyone" /> },
    { id: 'radio.off', w: 200, h: 64, stretch: true, render: () => <Radio value="b" checked={false} label="Nobody" /> },
    {
      id: 'radio-group', w: 240, h: 136, stretch: true,
      render: () => (
        <RadioGroup label="Who can reply" value="everyone" onValueChange={noop}
          options={[{ value: 'everyone', label: 'Everyone' }, { value: 'follow', label: 'People you follow' },
            { value: 'none', label: 'Nobody', disabled: true }]} />
      ),
    },
    ...(['underline', 'pill', 'filled'] as const).map((variant) => ({
      id: `tabs.${variant}`, w: 384, h: 72,
      render: () => (
        <Tabs variant={variant} value="inbox" onValueChange={noop} label="Mailbox">
          <TabsTrigger value="inbox" label="Inbox" count={3} />
          <TabsTrigger value="sent" label="Sent" />
          <TabsTrigger value="drafts" label="Drafts" leadingIcon={RiMailLine} />
        </Tabs>
      ),
    })),
    ...(['solid', 'plain'] as const).map((variant) => ({
      id: `segmented.${variant}`, w: 384, h: 72,
      render: () => (
        <SegmentedControl label="Period" type="radio" size="md" variant={variant} value="week" onValueChange={noop}>
          <SegmentedControlItem value="day"><SegmentedControlItemText>Day</SegmentedControlItemText></SegmentedControlItem>
          <SegmentedControlItem value="week"><SegmentedControlItemText>Week</SegmentedControlItemText></SegmentedControlItem>
          <SegmentedControlItem value="month"><SegmentedControlItemText>Month</SegmentedControlItemText></SegmentedControlItem>
        </SegmentedControl>
      ),
    })),
    ...(['xs', 'sm', 'md', 'lg'] as const).map((size, index) => ({
      id: `avatar.${size}`, w: 96, h: 88,
      render: () => <Avatar size={size} color={(['neutral', 'blue', 'lime', 'pink'] as const)[index]} initials="AL" />,
    })),
    { id: 'avatar.name', w: 96, h: 88, render: () => <Avatar size={48} name="Ada Lovelace" /> },
    ...(['solid', 'subtle', 'outlined'] as const).map((variant) => ({
      id: `badge.${variant}`, w: 112, h: 56,
      render: () => <Badge variant={variant} color="primary" content="New" />,
    })),
    { id: 'badge.count', w: 112, h: 56, render: () => <Badge color="error" content={12} /> },
    { id: 'badge.dot', w: 112, h: 56, render: () => <Badge dot color="success" /> },
    ...(['solid', 'subtle', 'outlined', 'inverted'] as const).map((variant) => ({
      id: `chip.${variant}`, w: 144, h: 64,
      render: () => <Chip variant={variant} color="primary">Design</Chip>,
    })),
    { id: 'chip.hue', w: 144, h: 64, render: () => <Chip variant="subtle" hue="lime">Lime</Chip> },
    { id: 'chip.close', w: 144, h: 64, render: () => <Chip variant="outlined" onClose={noop}>Filter</Chip> },
    {
      id: 'kbd', w: 160, h: 56,
      render: () => (
        <View style={{ flexDirection: 'row', gap: 6 }}>
          <Kbd>⌘</Kbd><Kbd>K</Kbd><Kbd size="sm">Esc</Kbd>
        </View>
      ),
    },
    { id: 'divider.single', w: 280, h: 48, stretch: true, render: () => <Divider /> },
    { id: 'divider.double', w: 280, h: 48, stretch: true, render: () => <Divider variant="double" /> },
    { id: 'divider.label', w: 280, h: 56, stretch: true, render: () => <Divider>or</Divider> },
    {
      id: 'icons.md', w: 284, h: 56,
      render: () => (
        <View style={{ flexDirection: 'row', gap: 12 }}>
          {[RiHomeLine, RiSearchLine, RiSettings3Line, RiUserLine, RiHeart3Line, RiNotification3Line, RiMailLine].map((Icon, index) => (
            <Icon key={index} size="md" />
          ))}
        </View>
      ),
    },
    {
      id: 'icons.sizes', w: 284, h: 80,
      render: () => (
        <View style={{ flexDirection: 'row', gap: 12, alignItems: 'center' }}>
          {(['xs', 'sm', 'md', 'lg', 'xl', '2xl'] as const).map((size) => <RiStarLine key={size} size={size} />)}
        </View>
      ),
    },
    {
      id: 'type.paragraph', w: 384, h: 136, stretch: true,
      render: () => <Text variant="body-regular">{PARAGRAPH}</Text>,
    },
  ],
  3: [
    {
      id: 'surface.default', w: 284, h: 168, stretch: true, inset: 20,
      render: () => (
        <Surface style={{ padding: 20, gap: 6 }}>
          <Text variant="title-3-semibold">Surface</Text>
          <Text variant="body-regular">Edge light and material.</Text>
        </Surface>
      ),
    },
    {
      id: 'surface.fill', w: 284, h: 168, stretch: true, inset: 20,
      render: () => (
        <Surface fill="#3b82f6" style={{ padding: 20, gap: 6 }}>
          <Text variant="title-3-semibold">Tinted</Text>
          <Text variant="body-regular">An opaque fill.</Text>
        </Surface>
      ),
    },
    {
      id: 'surface.radius-8', w: 284, h: 168, stretch: true, inset: 20,
      render: () => (
        <Surface radius={8} style={{ padding: 20, gap: 6 }}>
          <Text variant="title-3-semibold">Radius 8</Text>
          <Text variant="body-regular">Smaller corners.</Text>
        </Surface>
      ),
    },
    {
      id: 'surface.nested', w: 284, h: 168, stretch: true, inset: 20,
      render: () => (
        <Surface style={{ padding: 12 }}>
          <Card style={{ padding: 16 }}><Text variant="body-medium">Card in a surface</Text></Card>
        </Surface>
      ),
    },
    ...(['plain', 'solid', 'outline', 'subtle'] as const).map((appearance) => ({
      id: `card.${appearance}`, w: 284, h: 148, stretch: true, inset: 20,
      render: () => (
        <Card appearance={appearance}>
          <CardHeader><CardTitle>{appearance[0].toUpperCase() + appearance.slice(1)}</CardTitle>
            <CardDescription>Card appearance</CardDescription></CardHeader>
        </Card>
      ),
    })),
    ...(['none', 's', 'm', 'floating'] as const).map((elevation) => ({
      id: `card.elevation-${elevation}`, w: 284, h: 148, stretch: true, inset: 24,
      render: () => (
        <Card elevation={elevation}>
          <CardHeader><CardTitle>Elevation {elevation}</CardTitle><CardDescription>Shadow</CardDescription></CardHeader>
        </Card>
      ),
    })),
    ...(['hairline', 'thin', 'medium'] as const).map((border) => ({
      id: `card.border-${border}`, w: 284, h: 148, stretch: true, inset: 20,
      render: () => (
        <Card appearance="outline" border={border}>
          <CardHeader><CardTitle>Border {border}</CardTitle><CardDescription>Outline</CardDescription></CardHeader>
        </Card>
      ),
    })),
    {
      id: 'card.body', w: 284, h: 148, stretch: true, inset: 20,
      render: () => (
        <Card radius="radius-24">
          <CardBody><Text variant="body-regular">Radius 24, body only.</Text></CardBody>
        </Card>
      ),
    },
  ],
  4: [
    {
      id: 'grouped.filled', w: 384, h: 264, stretch: true,
      render: () => (
        <GroupedSection variant="filled" title="Account" footer="Changes apply immediately.">
          <GroupedSectionItem title="Email" value="ada@oxy.so" showChevron onPress={noop} />
          <GroupedSectionItem title="Notifications" rightElement={<Switch checked onCheckedChange={noop} accessibilityLabel="Notifications" />} />
          <GroupedSectionItem title="Sign out" destructive onPress={noop} />
        </GroupedSection>
      ),
    },
    {
      id: 'grouped.plain', w: 384, h: 264, stretch: true,
      render: () => (
        <GroupedSection variant="plain" separator="line" title="Privacy">
          <GroupedSectionItem title="Profile" description="Who can see it" value="Everyone" showChevron onPress={noop} />
          <GroupedSectionItem title="Messages" description="Who can write" value="Friends" showChevron onPress={noop} />
        </GroupedSection>
      ),
    },
    {
      id: 'settings-list', w: 384, h: 264, stretch: true,
      render: () => (
        <SettingsListGroup title="General">
          <SettingsListItem title="Language" value="English" showChevron onPress={noop} />
          <SettingsListItem title="Dark mode" rightElement={<Switch checked={false} onCheckedChange={noop} accessibilityLabel="Dark mode" />} />
          <SettingsListItem title="About" disabled />
        </SettingsListGroup>
      ),
    },
    { id: 'item.comfortable', w: 384, h: 88, stretch: true, render: () => <Item title="Comfortable" subtitle="Default density" leading={<Avatar size={32} initials="AL" />} trailing={<Badge content={3} color="primary" />} /> },
    { id: 'item.compact', w: 384, h: 72, stretch: true, render: () => <Item title="Compact" density="compact" leading={<RiStarLine size="md" />} /> },
    { id: 'item.selected', w: 384, h: 88, stretch: true, render: () => <Item title="Selected" subtitle="The current row" selected /> },
    { id: 'item.destructive', w: 384, h: 72, stretch: true, render: () => <Item title="Delete account" destructive /> },
    { id: 'item.disabled', w: 384, h: 72, stretch: true, render: () => <Item title="Unavailable" subtitle="Disabled" disabled /> },
    {
      id: 'table', w: 580, h: 232, stretch: true,
      render: () => (
        <Table accessibilityLabel="Customers">
          <TableHeader>
            <TableColumn sortDirection="ascending">Name</TableColumn>
            <TableColumn>Plan</TableColumn>
            <TableColumn align="end">Seats</TableColumn>
          </TableHeader>
          <TableBody>
            <TableRow><TableCell>Ada Lovelace</TableCell><TableCell>Pro</TableCell><TableCell align="end">12</TableCell></TableRow>
            <TableRow selected><TableCell>Alan Turing</TableCell><TableCell>Team</TableCell><TableCell align="end">48</TableCell></TableRow>
            <TableRow><TableCell>Grace Hopper</TableCell><TableCell>Free</TableCell><TableCell align="end">1</TableCell></TableRow>
          </TableBody>
        </Table>
      ),
    },
    {
      id: 'table.sm', w: 580, h: 196, stretch: true,
      render: () => (
        <Table size="sm" accessibilityLabel="Files">
          <TableHeader><TableColumn>File</TableColumn><TableColumn align="end">Size</TableColumn></TableHeader>
          <TableBody>
            <TableRow><TableCell>report.pdf</TableCell><TableCell align="end">2.4 MB</TableCell></TableRow>
            <TableRow><TableCell>photo.jpg</TableCell><TableCell align="end">840 KB</TableCell></TableRow>
          </TableBody>
        </Table>
      ),
    },
  ],
  5: typeCells(),
  6: [
    {
      id: 'sidebar.card', w: 284, h: 400, stretch: true,
      render: () => (
        <View style={{ height: 360 }}>
          <Sidebar surface="card" size="md" items={SIDEBAR_ITEMS} selected="home" showSearch={false} showThemeToggle={false} />
        </View>
      ),
    },
    {
      id: 'sidebar.plain', w: 284, h: 400, stretch: true,
      render: () => (
        <View style={{ height: 360 }}>
          <Sidebar surface="plain" size="md" items={SIDEBAR_ITEMS} selected="mail" showSearch={false} showThemeToggle={false} />
        </View>
      ),
    },
    {
      id: 'sidebar.rail', w: 104, h: 400,
      render: () => (
        <View style={{ height: 360 }}>
          <Sidebar surface="card" variant="rail" items={SIDEBAR_ITEMS} selected="search" showSearch={false} showThemeToggle={false} />
        </View>
      ),
    },
    { id: 'sidebar-item.selected', w: 264, h: 64, stretch: true, render: () => <SidebarItem icon={RiHomeLine} label="Home" selected /> },
    { id: 'sidebar-item', w: 264, h: 64, stretch: true, render: () => <SidebarItem icon={RiMailLine} label="Mail" /> },
    { id: 'sidebar-item.neutral', w: 264, h: 64, stretch: true, render: () => <SidebarItem icon={RiUserLine} label="People" selected selectedAppearance="neutral" /> },
    {
      id: 'toolbar.island', w: 480, h: 96, inset: 20,
      render: () => (
        <Surface style={{ padding: 4 }}>
          <Toolbar size="sm">
            <IconButton leadingIcon={RiAddLine} accessibilityLabel="New" size="sm" />
            <IconButton leadingIcon={RiSearchLine} accessibilityLabel="Search" size="sm" />
            <Button size="sm" appearance="subtle">Share</Button>
          </Toolbar>
        </Surface>
      ),
    },
  ],
};

// The overlays (pages 7 and 8) open at once and stay open: a dialog, a popover, a tooltip and a
// toast that never closes by itself, drawn where each platform draws them (a portal in a browser;
// on OxyOS a sheet or a compositor popup), each page one cell over the window.
function DialogPage() {
  return (
    <Dialog startOpen title="Sign out?" description="You'll need your password to sign in again."
      actions={[{ label: 'Sign out', color: 'destructive' }, { label: 'Cancel', color: 'cancel' }]} />
  );
}

function OverlaysPage() {
  React.useEffect(() => {
    toast.success('Saved to your library', { duration: Infinity });
  }, []);
  return (
    <View style={{ flexDirection: 'row', gap: 240, paddingTop: 40, paddingLeft: 40 }}>
      <Popover defaultOpen>
        <PopoverTrigger asChild label="Design team">
          <Button appearance="outline" tone="neutral">Design team</Button>
        </PopoverTrigger>
        <PopoverContent label="Design team menu" align="start">
          <View style={{ gap: 4, padding: 4 }}>
            <Text variant="headline-semibold">Design team</Text>
            <Text variant="body-regular">hi@example.com</Text>
          </View>
        </PopoverContent>
      </Popover>
      <Tooltip visible onVisibleChange={noop} position="bottom">
        <TooltipTrigger><Text variant="body-medium">Copy link</Text></TooltipTrigger>
        <TooltipTextBubble>Copies the link to your clipboard</TooltipTextBubble>
      </Tooltip>
    </View>
  );
}

// The app shell (page 9) as an application lays it out: the whole window, its sidebar, its
// header's title and actions, and two panel islands.
function AppShellPage() {
  return (
    <AppShell
      panels
      scroll="fixed"
      contentMaxWidth="none"
      title="Bloom fidelity · page 9"
      actions={<Button size="md" appearance="outline" tone="neutral" leadingIcon={RiAddLine}>New</Button>}
      sidebar={{ items: SIDEBAR_ITEMS, selected: 'mail' }}
    >
      <AppShellPanels>
        <AppShellPanel id="list" width={360} minWidth={280}>
          <View style={{ padding: 12, gap: 4 }}>
            <Item title="Ada Lovelace" subtitle="The analytical engine" leading={<Avatar size={32} initials="AL" />} selected />
            <Item title="Alan Turing" subtitle="On computable numbers" leading={<Avatar size={32} color="blue" initials="AT" />} />
            <Item title="Grace Hopper" subtitle="A compiler" leading={<Avatar size={32} color="lime" initials="GH" />} />
          </View>
        </AppShellPanel>
        <AppShellPanel id="message" minWidth={320}>
          <View style={{ padding: 24, gap: 8 }}>
            <Text variant="title-2-semibold">The analytical engine</Text>
            <Text variant="body-regular">It weaves algebraic patterns as the loom weaves flowers.</Text>
          </View>
        </AppShellPanel>
      </AppShellPanels>
    </AppShell>
  );
}

type OverlayPage = { render: () => React.ReactNode; full?: boolean };
const OVERLAY_PAGES: Record<number, OverlayPage> = {
  7: { render: () => <DialogPage /> },
  8: { render: () => <OverlaysPage /> },
  9: { render: () => <AppShellPage />, full: true },
};

const HEADER = 40;
const MARGIN = 16;
const GAP = 8;

type Placed = Cell & { x: number; y: number };

/** The cells in rows, left to right, each row as tall as its tallest cell. */
function place(cells: Cell[], width: number, height: number): Placed[] {
  const placed: Placed[] = [];
  let x = MARGIN;
  let y = HEADER;
  let row = 0;
  for (const cell of cells) {
    if (x + cell.w > width - MARGIN) {
      x = MARGIN;
      y += row + GAP;
      row = 0;
    }
    if (y + cell.h > height - MARGIN) console.warn(`[fidelity] cell ${cell.id} runs past the page`);
    placed.push({ ...cell, x, y });
    x += cell.w + GAP;
    row = Math.max(row, cell.h);
  }
  return placed;
}

type Layout = { x: number; y: number; w: number; h: number; cw: number; ch: number; label: string };

function Gallery({ page }: { page: number }) {
  const theme = useTheme();
  const spec = PAGES.find((p) => p.page === page) ?? PAGES[0];
  const overlay = OVERLAY_PAGES[spec.page];
  const cells = React.useMemo(
    (): Placed[] => {
      if (overlay?.full) {
        // The whole window, as an application has it.
        return [{ id: `overlay.page${spec.page}`, x: 0, y: 0, w: spec.width, h: spec.height, fill: true, render: overlay.render }];
      }
      if (overlay) {
        return place([{ id: `overlay.page${spec.page}`, w: spec.width - 2 * MARGIN, h: spec.height - HEADER - MARGIN,
          stretch: true, inset: 0, render: overlay.render }], spec.width, spec.height);
      }
      return place(PAGE_CELLS[spec.page] ?? [], spec.width, spec.height);
    },
    [overlay, spec],
  );
  const measured = React.useRef<Record<string, Layout>>({});
  const timer = React.useRef<ReturnType<typeof setTimeout> | null>(null);
  const report = React.useCallback(() => {
    if (Object.keys(measured.current).length < cells.length) return;
    if (timer.current) clearTimeout(timer.current);
    // Once nothing has moved for a moment: what the frame shows.
    timer.current = setTimeout(() => {
      const state = { page: spec.page, width: spec.width, height: spec.height, ready: true, cells: measured.current };
      if (Platform.OS === 'web') (globalThis as { __fidelity?: unknown }).__fidelity = JSON.parse(JSON.stringify(state));
      else console.log(`[fidelity] ${JSON.stringify(state)}`);
    }, 400);
  }, [cells.length, spec]);
  return (
    <View testID="fidelity-page" style={{ width: spec.width, height: spec.height, backgroundColor: theme.colors.background, overflow: 'hidden' }}>
      {overlay?.full ? null : (
        <View style={{ position: 'absolute', left: MARGIN, top: 12, flexDirection: 'row', gap: 8 }}>
          <Text variant="caption-1-medium" style={{ color: theme.colors.textSecondary }}>{`Bloom fidelity · page ${spec.page}`}</Text>
          <Text variant="caption-1-regular" style={{ color: theme.colors.textSecondary }}>{spec.title}</Text>
        </View>
      )}
      {cells.map((cell) => {
        const inset = cell.inset ?? 12;
        return (
          <View
            key={cell.id}
            testID={cell.id}
            nativeID={cell.id}
            style={{ position: 'absolute', left: cell.x, top: cell.y, width: cell.w, height: cell.h, overflow: 'hidden' }}
          >
            <View
              style={cell.fill
                ? { position: 'absolute', left: 0, top: 0, right: 0, bottom: 0 }
                : { position: 'absolute', left: inset, top: inset, ...(cell.stretch ? { right: inset } : null) }}
              onLayout={(event) => {
                const { width, height } = event.nativeEvent.layout;
                measured.current[cell.id] = { x: cell.x, y: cell.y, w: cell.w, h: cell.h, cw: width, ch: height, label: cell.id };
                report();
              }}
            >
              {cell.render()}
            </View>
          </View>
        );
      })}
    </View>
  );
}

export default function FidelityGallery({ page = 1 }: { page?: number }) {
  return (
    <SafeAreaProvider>
      <BloomProvider platform={fidelityPlatform} mode="system">
        <PortalProvider>
          <Gallery page={page} />
          <PortalOutlet />
          {page === 8 ? <ToastOutlet /> : null}
        </PortalProvider>
      </BloomProvider>
    </SafeAreaProvider>
  );
}

/** The page as a module of its own (FidelityGalleryN), for the host, which gives a module no props. */
export function fidelityPage(page: number) {
  return function FidelityGalleryPage() {
    return <FidelityGallery page={page} />;
  };
}
