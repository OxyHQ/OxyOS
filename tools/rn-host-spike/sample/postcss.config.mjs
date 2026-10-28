// Tailwind v4 through Expo's CSS pipeline, as every Oxy app compiles its global.css
// (NativeWind v5's setup); react-native-css turns the result into native styles.
export default { plugins: { '@tailwindcss/postcss': {} } };
