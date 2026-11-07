/**
 * General › Transfer or Reset (docs/adr/0050-recovery-and-erasing.mdx), as a Mac's: *Erase All
 * Content and Settings…* and its Erase Assistant, one sheet that goes from what will be erased to
 * the administrator's password to the erase itself. The platform decides everything (who may, the
 * password, arming the erase, the restart); the page shows what it is told and says what came of
 * it. Where the platform has no account session, nothing is offered.
 */
import * as React from "react";
import { AccessibilityInfo, TextInput, View } from "react-native";
import { Dialog, type DialogAction } from "@oxy.so/bloom/dialog";
import { GroupedSection, GroupedSectionItem } from "@oxy.so/bloom/grouped-section";
import { Loading } from "@oxy.so/bloom/loading";
import { TextField, TextFieldHint, TextFieldInput } from "@oxy.so/bloom/text-field";
import { Text } from "@oxy.so/bloom/typography";
import { initialsOf } from "./account";
import { AccountPicture } from "./AccountPicture";
import {
  administratorOf,
  eraseAvailability,
  refusalMessage,
  signOutLine,
  type ErasePreview,
} from "./erase";
import type { Messages } from "./messages";
import type { SettingsActions, SettingsSnapshot } from "./model";

type Props = { snapshot: SettingsSnapshot; actions: SettingsActions; strings: Messages; locale: string };

/** Where the Erase Assistant is: what will be erased, the password, or erasing (no way back). */
type Step = { kind: "what"; preview: ErasePreview } | { kind: "password"; preview: ErasePreview } | { kind: "erasing" };

export function TransferResetPage({ actions, strings }: Props) {
  const words = strings.transferReset;
  const erase = actions.erase;
  // undefined while asked; null when the platform could not say.
  const [preview, setPreview] = React.useState<ErasePreview | null | undefined>(undefined);
  const [step, setStep] = React.useState<Step | null>(null);
  const [password, setPassword] = React.useState("");
  const [fieldError, setFieldError] = React.useState<string | null>(null);
  const [stepError, setStepError] = React.useState<string | null>(null);
  const [working, setWorking] = React.useState(false);
  const field = React.useRef<React.ComponentRef<typeof TextInput>>(null);
  const asked = React.useRef(0);

  // What erasing would do, asked as the page opens and again as the assistant starts: the accounts
  // and whether the computer is online are those of now.
  const ask = React.useCallback(async () => {
    if (!erase) return null;
    const turn = ++asked.current;
    const told = await erase.preview();
    if (turn === asked.current) setPreview(told);
    return told;
  }, [erase]);
  React.useEffect(() => {
    void ask();
  }, [ask]);

  if (!erase) {
    return (
      <GroupedSection>
        <GroupedSectionItem title={words.unavailableHere} titleNumberOfLines={0} testID="transfer-reset-unavailable" />
      </GroupedSection>
    );
  }

  const availability = preview === undefined ? null : eraseAvailability(preview);
  const reason =
    availability === "none"
      ? words.unavailableHere
      : availability === "noRecovery"
        ? words.noRecovery
        : availability === "notAdministrator"
          ? words.notAdministrator
          : undefined;

  const start = async () => {
    const told = await ask();
    if (eraseAvailability(told) === "ready" && told) setStep({ kind: "what", preview: told });
  };
  const close = () => {
    if (working || step?.kind === "erasing") return;
    setStep(null);
    setPassword("");
    setFieldError(null);
    setStepError(null);
  };
  const submit = async () => {
    if (working || step?.kind !== "password" || password === "") return;
    setWorking(true);
    setFieldError(null);
    setStepError(null);
    const typed = password;
    setPassword("");
    const outcome = await erase.eraseAll(typed);
    setWorking(false);
    if ("erasing" in outcome) {
      setStep({ kind: "erasing" });
      AccessibilityInfo.announceForAccessibility(`${words.erasingTitle} ${words.erasingDescription}`);
      return;
    }
    const said = refusalMessage(outcome, words);
    setFieldError(said.field);
    setStepError(said.step);
    // The field keeps the keyboard's focus, as a Mac's password sheet does after a wrong one.
    field.current?.focus();
  };

  const dialog = (() => {
    if (!step) return null;
    if (step.kind === "erasing") {
      return {
        title: words.erasingTitle,
        description: words.erasingDescription,
        actions: [] as DialogAction[],
        content: (
          <View style={{ alignItems: "center", paddingVertical: 12 }}>
            <Loading testID="erase-assistant-erasing" />
          </View>
        ),
      };
    }
    const { preview: shown } = step;
    if (step.kind === "what") {
      return {
        title: words.title,
        description: undefined,
        actions: [
          { label: words.cancel, color: "cancel", onPress: close, testID: "erase-assistant-cancel" },
          {
            label: words.continue,
            color: "default",
            shouldCloseOnPress: false,
            initialFocus: true,
            onPress: () => setStep({ kind: "password", preview: shown }),
            testID: "erase-assistant-continue",
          },
        ] as DialogAction[],
        content: (
          <View style={{ gap: 12, paddingBottom: 12 }}>
            <Text variant="body-regular">{words.whatIntro}</Text>
            <GroupedSection>
              {shown.accounts.map((account, index) => (
                <GroupedSectionItem
                  key={`${index}-${account.name}`}
                  icon={<AccountPicture picture={null} initials={initialsOf(account.name)} size={28} />}
                  title={account.name}
                  description={`${words.accountItem}\n${signOutLine(account, shown, words)}`}
                  descriptionLines={0}
                  testID={`erase-assistant-account-${index}`}
                />
              ))}
              <GroupedSectionItem title={words.wifiItem} titleNumberOfLines={0} testID="erase-assistant-settings" />
            </GroupedSection>
            <Text variant="body-regular" style={{ opacity: 0.7 }}>
              {words.staysInstalled}
            </Text>
          </View>
        ),
      };
    }
    return {
      title: words.title,
      description: words.passwordDescription(administratorOf(shown).name),
      actions: [
        { label: words.cancel, color: "cancel", onPress: close, disabled: working, testID: "erase-assistant-cancel" },
        {
          label: words.eraseConfirm,
          color: "destructive",
          shouldCloseOnPress: false,
          loading: working,
          disabled: password === "",
          onPress: () => void submit(),
          testID: "erase-assistant-erase",
        },
      ] as DialogAction[],
      content: (
        <View style={{ gap: 8, paddingBottom: 12 }}>
          <TextField invalid={fieldError !== null}>
            <TextFieldInput
              inputRef={field}
              label={words.password}
              placeholder={words.password}
              value={password}
              onChangeText={(text) => {
                setPassword(text);
                setFieldError(null);
              }}
              secureTextEntry
              autoFocus
              autoCapitalize="none"
              autoCorrect={false}
              editable={!working}
              returnKeyType="done"
              submitBehavior="submit"
              onSubmitEditing={() => void submit()}
              testID="erase-assistant-password"
            />
          </TextField>
          {fieldError ? (
            <TextFieldHint invalid>{fieldError}</TextFieldHint>
          ) : null}
          {stepError ? (
            <Text variant="body-regular" accessibilityRole="alert" testID="erase-assistant-error">
              {stepError}
            </Text>
          ) : null}
        </View>
      ),
    };
  })();

  return (
    <View style={{ gap: 16 }}>
      <Text style={{ fontSize: 13 }}>{words.description}</Text>
      <GroupedSection footer={reason}>
        <GroupedSectionItem
          title={words.erase}
          onPress={() => void start()}
          destructive
          disabled={availability !== "ready"}
          showChevron={false}
          accessibilityRole="button"
          accessibilityHint={reason}
          testID="transfer-reset-erase"
        />
      </GroupedSection>
      <Dialog
        open={dialog !== null}
        onClose={close}
        title={dialog?.title}
        description={dialog?.description}
        label={dialog?.title}
        actions={dialog?.actions}
        // Nothing dismisses it while the erase is asked or under way: there is no way back.
        dismissOnBackdrop={!working && step?.kind !== "erasing"}
        testID="erase-assistant"
      >
        {dialog?.content}
      </Dialog>
    </View>
  );
}
