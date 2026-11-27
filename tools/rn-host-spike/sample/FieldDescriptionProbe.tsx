/**
 * A field said with its description (scenarios/field-description.json, docs/adr/0039-keyboard-access.mdx):
 * a Bloom `TextField` in a `Field` with a description, and a button that shows the field's error
 * in its place. On the web Bloom wires both with `aria-describedby`; React Native drops that prop,
 * so Bloom gives the field the described texts as its `accessibilityHint`, which the screen reader
 * says after the field, politely, as VoiceOver says a control's help. The button's tooltip is its
 * help too, as a Mac's help tag is.
 */
import * as React from 'react';
import { View } from 'react-native';
import { BloomProvider } from '@oxy.so/bloom/provider';
import { Button } from '@oxy.so/bloom/button';
import { Field } from '@oxy.so/bloom/field';
import { TextField, TextFieldInput } from '@oxy.so/bloom/text-field';
import { Tooltip, TooltipTextBubble, TooltipTrigger } from '@oxy.so/bloom/tooltip';
import { oxyosSurfaces } from './oxyos/platform-surface';

export default function FieldDescriptionProbe() {
  const [name, setName] = React.useState('');
  const [taken, setTaken] = React.useState(false);
  const [tip, setTip] = React.useState(false);
  return (
    <BloomProvider platform={oxyosSurfaces} locale="es">
      <View style={{ padding: 24, gap: 16 }}>
        <Field
          label="Nombre de usuario"
          description="Entre 3 y 20 letras o números"
          error={taken ? 'Ese nombre ya está en uso' : null}
        >
          <TextField>
            <TextFieldInput
              testID="user"
              label="Nombre de usuario"
              placeholder="usuario"
              value={name}
              onChangeText={setName}
            />
          </TextField>
        </Field>
        <Tooltip visible={tip} onVisibleChange={setTip} position="bottom" delay={0}>
          <TooltipTrigger>
            <Button testID="check" onPress={() => setTaken((shown) => !shown)}>
              Comprobar
            </Button>
          </TooltipTrigger>
          <TooltipTextBubble>Comprueba si el nombre está libre</TooltipTextBubble>
        </Tooltip>
      </View>
    </BloomProvider>
  );
}
