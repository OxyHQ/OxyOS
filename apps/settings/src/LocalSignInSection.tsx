/**
 * Settings › Oxy Account › Sign-In on This Computer (docs/adr/0020-oxy-account-login.mdx, decision
 * 2), as Windows' sign-in options and a Mac's password and Touch ID: the account's PIN (set up,
 * changed, removed) and whether its password signs in here, each change in a sheet that asks the
 * person to prove it (the password, or the current PIN) and, for a new PIN, to type it twice. The
 * accounts' service decides; the section shows what it is told and says what came of it.
 */
import * as React from "react";
import { AccessibilityInfo, View } from "react-native";
import { Dialog, type DialogAction } from "@oxy.so/bloom/dialog";
import { GroupedSection, GroupedSectionItem } from "@oxy.so/bloom/grouped-section";
import { Switch } from "@oxy.so/bloom/switch";
import { TextField, TextFieldHint, TextFieldInput } from "@oxy.so/bloom/text-field";
import {
  localRefusal,
  proofFor,
  usablePin,
  type LocalChange,
  type LocalSignIn,
  type LocalSignInActions,
} from "./local-sign-in";
import type { Messages } from "./messages";

type Fields = { proof: string; pin: string; again: string };
const EMPTY: Fields = { proof: "", pin: "", again: "" };

export function LocalSignInSection({ actions, strings }: { actions: LocalSignInActions; strings: Messages }) {
  const words = strings.localSignIn;
  // undefined while asked; null when the platform could not say: then nothing is offered.
  const [local, setLocal] = React.useState<LocalSignIn | null | undefined>(undefined);
  const [change, setChange] = React.useState<LocalChange | null>(null);
  const [fields, setFields] = React.useState<Fields>(EMPTY);
  const [error, setError] = React.useState<string | null>(null);
  const [working, setWorking] = React.useState(false);
  React.useEffect(() => {
    let current = true;
    actions
      .read()
      .then((read) => current && setLocal(read))
      .catch(() => current && setLocal(null));
    return () => {
      current = false;
    };
  }, [actions]);
  if (!local) return null;
  const open = (next: LocalChange) => {
    setFields(EMPTY);
    setError(null);
    setChange(next);
  };
  const close = () => {
    if (working) return;
    setChange(null);
    setFields(EMPTY);
    setError(null);
  };
  const proof = change ? proofFor(change, local) : "password";
  const newPin = change === "setPin";
  const ready =
    fields.proof !== "" && (!newPin || (fields.pin !== "" && fields.again !== ""));
  const submit = async () => {
    if (!change || !ready || working) return;
    if (newPin && !usablePin(fields.pin)) return setError(words.unusablePin);
    if (newPin && fields.pin !== fields.again) return setError(words.mismatch);
    setWorking(true);
    setError(null);
    const outcome = await actions
      .change(change, {
        ...(newPin ? { pin: fields.pin } : {}),
        ...(proof === "password" ? { password: fields.proof } : { currentPin: fields.proof }),
      })
      .catch(() => ({ failed: "unavailable" }) as const);
    setWorking(false);
    if ("localSignIn" in outcome) {
      setLocal(outcome.localSignIn);
      setChange(null);
      setFields(EMPTY);
      return;
    }
    const said = localRefusal(outcome, strings);
    setError(said);
    setFields((typed) => ({ ...typed, proof: "" }));
    AccessibilityInfo.announceForAccessibility(said);
  };
  const title =
    change === "setPin"
      ? local.pin === "none"
        ? words.setUpTitle
        : words.changeTitle
      : change === "removePin"
        ? words.removeTitle
        : change === "passwordOff"
          ? words.passwordOffTitle
          : words.passwordOnTitle;
  const description =
    change === "removePin"
      ? words.removeDescription
      : change === "passwordOff"
        ? words.passwordOffDescription
        : change === "passwordOn"
          ? words.passwordOnDescription
          : undefined;
  const confirm =
    change === "removePin" ? words.removeConfirm : change === "passwordOff" ? words.turnOff : change === "passwordOn" ? words.turnOn : words.save;
  const field = (
    key: keyof Fields,
    label: string,
    digits: boolean,
    testID: string,
    autoFocus = false,
  ) => (
    <TextField invalid={error !== null && key === "proof"}>
      <TextFieldInput
        label={label}
        placeholder={label}
        value={fields[key]}
        onChangeText={(text) => {
          setFields((typed) => ({ ...typed, [key]: digits ? text.replace(/[^0-9]/g, "") : text }));
          setError(null);
        }}
        secureTextEntry
        autoFocus={autoFocus}
        autoCapitalize="none"
        autoCorrect={false}
        keyboardType={digits ? "number-pad" : "default"}
        maxLength={digits ? 16 : 512}
        editable={!working}
        submitBehavior="submit"
        onSubmitEditing={() => void submit()}
        testID={testID}
      />
    </TextField>
  );
  const pinState = local.pin === "set" ? words.pinSet : local.pin === "locked" ? words.pinLocked : words.pinNone;
  return (
    <>
      <GroupedSection title={words.title} footer={words.footer}>
        <GroupedSectionItem
          title={words.pin}
          description={pinState}
          descriptionLines={0}
          testID="local-sign-in-pin"
        />
        <GroupedSectionItem
          title={local.pin === "none" ? words.setUp : words.change}
          onPress={() => open("setPin")}
          showChevron={false}
          accessibilityRole="button"
          testID="local-sign-in-set-pin"
        />
        {local.pin !== "none" && local.password ? (
          <GroupedSectionItem
            title={words.remove}
            onPress={() => open("removePin")}
            destructive
            showChevron={false}
            accessibilityRole="button"
            testID="local-sign-in-remove-pin"
          />
        ) : null}
        <GroupedSectionItem
          title={words.password}
          description={local.pin === "none" ? words.passwordNeedsPin : words.passwordDescription}
          descriptionLines={0}
          testID="local-sign-in-password"
          rightElement={
            <Switch
              checked={local.password}
              disabled={local.pin === "none" && local.password}
              onCheckedChange={(on: boolean) => open(on ? "passwordOn" : "passwordOff")}
              accessibilityLabel={words.password}
              testID="local-sign-in-password-switch"
            />
          }
        />
      </GroupedSection>
      <Dialog
        open={change !== null}
        onClose={close}
        title={title}
        description={description}
        label={title}
        actions={
          [
            { label: words.cancel, color: "cancel", onPress: close, disabled: working, testID: "local-sign-in-cancel" },
            {
              label: confirm,
              color: change === "removePin" || change === "passwordOff" ? "destructive" : "default",
              shouldCloseOnPress: false,
              loading: working,
              disabled: !ready,
              onPress: () => void submit(),
              testID: "local-sign-in-confirm",
            },
          ] as DialogAction[]
        }
        dismissOnBackdrop={!working}
        testID="local-sign-in-sheet"
      >
        <View style={{ gap: 8, paddingBottom: 12 }}>
          {field(
            "proof",
            proof === "password" ? words.currentPassword : words.currentPin,
            proof === "pin",
            "local-sign-in-proof",
            true,
          )}
          {newPin ? field("pin", words.newPin, true, "local-sign-in-new-pin") : null}
          {newPin ? field("again", words.confirmPin, true, "local-sign-in-again") : null}
          {error ? (
            <View testID="local-sign-in-error" accessibilityRole="alert">
              <TextFieldHint invalid>{error}</TextFieldHint>
            </View>
          ) : null}
        </View>
      </Dialog>
    </>
  );
}
