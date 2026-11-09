/**
 * General › Login Items & Extensions (docs/adr/0060-login-items-and-background-activity.mdx), as a
 * Mac's: *Open at Login*, the person's one list of the applications the session opens as it starts
 * (the dock's menu's Options › Open at Login changes the same list, and each shows the other's
 * change at once), an application added with + from the registry's and removed with −; and *App
 * Background Activity*, each installed application that registered work to run in the background,
 * whether it runs there now, and a switch the person turns it off with, which the session applies
 * at once. The platform decides and keeps everything; the page shows what it is told.
 */
import * as React from "react";
import { AccessibilityInfo, View } from "react-native";
import { Button } from "@oxy.so/bloom/button";
import { ChoiceList } from "@oxy.so/bloom/choice-list";
import { Dialog } from "@oxy.so/bloom/dialog";
import { GroupedSection, GroupedSectionItem } from "@oxy.so/bloom/grouped-section";
import { RiAddLine, RiSubtractLine } from "@oxy.so/bloom/icons";
import { Switch } from "@oxy.so/bloom/switch";
import { useTheme } from "@oxy.so/bloom/theme";
import { Text } from "@oxy.so/bloom/typography";
import {
  addableApplications,
  backgroundRows,
  loginItemRows,
  selectedAfterRemoving,
  withLoginItem,
  withoutLoginItem,
  type LoginItemRow,
} from "./login-items";
import type { Messages } from "./messages";
import type { SettingsActions, SettingsSnapshot } from "./model";

type Props = { snapshot: SettingsSnapshot; actions: SettingsActions; strings: Messages; locale: string };

const PAGE = { gap: 24 } as const;
const ROW_OPTION = { alignItems: "stretch", borderRadius: 10, paddingHorizontal: 4 } as const;

/** One application's row, drawn: its name and its kind, as a Mac's list shows them. Decorative: the
    list's option is the control, named by its label. */
function ItemRow({ name, kind, selected }: { name: string; kind: string; selected: boolean }) {
  const theme = useTheme();
  const ink = selected ? theme.colors.primaryForeground : theme.colors.text;
  return (
    <View
      style={{
        flexDirection: "row",
        alignItems: "center",
        justifyContent: "space-between",
        gap: 12,
        paddingHorizontal: 10,
        paddingVertical: 7,
        borderRadius: 10,
        backgroundColor: selected ? theme.colors.primary : "transparent",
      }}
    >
      <Text numberOfLines={1} style={{ color: ink, fontSize: 14, flexShrink: 1 }}>
        {name}
      </Text>
      <Text style={{ color: selected ? ink : theme.colors.textSecondary, fontSize: 12 }}>{kind}</Text>
    </View>
  );
}

function OpenAtLogin({ snapshot, actions, strings, locale }: Props) {
  const words = strings.loginItems;
  const listed = snapshot.preferences.openAtLogin;
  const applications = snapshot.applications ?? [];
  const rows = loginItemRows(listed, applications);
  const [selected, setSelected] = React.useState<string | null>(listed[0] ?? null);
  const shown = selected !== null && listed.includes(selected) ? selected : (listed[0] ?? null);
  const canAdd = addableApplications(listed, applications, locale);
  const [adding, setAdding] = React.useState(false);
  const [choice, setChoice] = React.useState<string | null>(null);
  const plus = React.useRef<React.ComponentRef<typeof View>>(null);
  const minusFocused = React.useRef(false);
  // A change asked for, said once the session's list shows it; an application added is selected
  // then, as a Mac selects what it adds (until then the selection stays where it was).
  const asked = React.useRef<{ kind: "added" | "removed"; row: LoginItemRow } | null>(null);
  React.useEffect(() => {
    const change = asked.current;
    if (!change) return;
    const there = listed.includes(change.row.id);
    if (change.kind === "added" ? !there : there) return;
    asked.current = null;
    if (change.kind === "added") setSelected(change.row.id);
    AccessibilityInfo.announceForAccessibility(words[change.kind](change.row.name));
  }, [listed, words]);

  const add = () => {
    const app = canAdd.find((candidate) => candidate.id === choice);
    if (!app) return;
    asked.current = { kind: "added", row: app };
    actions.setPreference("openAtLogin", withLoginItem(listed, app.id));
  };
  const remove = () => {
    const row = rows.find((candidate) => candidate.id === shown);
    if (!row) return;
    const next = selectedAfterRemoving(listed, row.id);
    asked.current = { kind: "removed", row };
    setSelected(next);
    actions.setPreference("openAtLogin", withoutLoginItem(listed, row.id));
    // The − button goes disabled under the keyboard with the list empty: the keyboard goes to +,
    // as it would otherwise be left on nothing.
    if (next === null && minusFocused.current && plus.current) {
      AccessibilityInfo.sendAccessibilityEvent(plus.current as never, "focus");
    }
  };
  const shownRow = rows.find((row) => row.id === shown);
  return (
    <View>
      <GroupedSection title={words.openAtLogin} footer={words.openAtLoginFooter}>
        <GroupedSectionItem
          title={words.openAtLogin}
          testID="login-items"
          content={
            <View style={{ gap: 8 }}>
              {rows.length === 0 ? (
                <Text style={{ fontSize: 13, paddingHorizontal: 14, paddingVertical: 7 }} testID="login-items-none">
                  {words.none}
                </Text>
              ) : (
                <ChoiceList<string>
                  label={words.openAtLoginList}
                  value={shown ?? rows[0].id}
                  onValueChange={setSelected}
                  orientation="vertical"
                  style={{ gap: 2 }}
                  optionStyle={ROW_OPTION}
                  options={rows.map((row) => ({ value: row.id, label: row.name, testID: `login-item-${row.id}` }))}
                  renderContent={(option, { selected: isSelected }) => (
                    <ItemRow name={option.label} kind={words.kind} selected={isSelected} />
                  )}
                  testID="login-items-list"
                />
              )}
              <View style={{ flexDirection: "row", gap: 4 }}>
                <Button
                  ref={plus}
                  size="sm"
                  appearance="plain"
                  tone="neutral"
                  iconOnly
                  icon={RiAddLine}
                  accessibilityLabel={words.add}
                  disabled={canAdd.length === 0}
                  onPress={() => {
                    setChoice(canAdd[0]?.id ?? null);
                    setAdding(true);
                  }}
                  testID="login-items-add"
                />
                <Button
                  size="sm"
                  appearance="plain"
                  tone="neutral"
                  iconOnly
                  icon={RiSubtractLine}
                  accessibilityLabel={shownRow ? words.removeItem(shownRow.name) : words.remove}
                  disabled={!shownRow}
                  onFocus={() => {
                    minusFocused.current = true;
                  }}
                  onBlur={() => {
                    minusFocused.current = false;
                  }}
                  onPress={remove}
                  testID="login-items-remove"
                />
              </View>
            </View>
          }
        />
      </GroupedSection>
      <Dialog
        open={adding}
        onClose={() => setAdding(false)}
        title={words.addTitle}
        description={canAdd.length === 0 ? words.nothingToAdd : words.addDescription}
        actions={[
          { label: words.cancel, color: "cancel", testID: "login-items-add-cancel" },
          {
            label: words.confirmAdd,
            color: "default",
            disabled: choice === null,
            onPress: add,
            testID: "login-items-add-confirm",
          },
        ]}
        testID="login-items-add-sheet"
      >
        {choice !== null ? (
          <ChoiceList<string>
            label={words.addList}
            value={choice}
            onValueChange={setChoice}
            orientation="vertical"
            style={{ gap: 2 }}
            optionStyle={ROW_OPTION}
            options={canAdd.map((app) => ({ value: app.id, label: app.name, testID: `login-items-add-${app.id}` }))}
            renderContent={(option, { selected: isSelected }) => (
              <ItemRow name={option.label} kind={words.kind} selected={isSelected} />
            )}
            testID="login-items-add-list"
          />
        ) : null}
      </Dialog>
    </View>
  );
}

function BackgroundActivity({ snapshot, actions, strings, locale }: Props) {
  const words = strings.loginItems;
  const act = actions.privacy;
  if (snapshot.privacy === undefined || !act) {
    return (
      <GroupedSection title={words.background}>
        <GroupedSectionItem title={words.unavailable} titleNumberOfLines={0} testID="background-activity-unavailable" />
      </GroupedSection>
    );
  }
  const rows = backgroundRows(snapshot.privacy, locale);
  return (
    <GroupedSection title={words.background} footer={words.backgroundFooter}>
      {rows.length === 0 ? (
        <GroupedSectionItem title={words.backgroundNone} titleNumberOfLines={0} testID="background-activity-none" />
      ) : (
        rows.map((row) => (
          <GroupedSectionItem
            key={row.id}
            title={row.name}
            description={row.running ? words.running : row.allowed ? words.notRunning : words.off}
            testID={`background-activity-${row.id}`}
            rightElement={
              // Named with its application, as the switch is reached alone (Tab, the reader's focus).
              <Switch
                checked={row.allowed}
                onCheckedChange={(allowed: boolean) => act.decide(row.id, "background", allowed)}
                accessibilityLabel={words.allowFor(row.name)}
                testID={`background-activity-${row.id}-switch`}
              />
            }
          />
        ))
      )}
    </GroupedSection>
  );
}

export function LoginItemsPage(props: Props) {
  return (
    <View style={PAGE}>
      <OpenAtLogin {...props} />
      <BackgroundActivity {...props} />
    </View>
  );
}
