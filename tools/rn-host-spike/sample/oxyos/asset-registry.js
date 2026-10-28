// Metro's asset modules call `require(assetRegistryPath).registerAsset(...)`. React Native 0.88
// keeps the registry `AssetRegistry` export reads (and expo-asset uses) as a named export.
module.exports = require('react-native/src/private/assets/AssetRegistry').AssetRegistry;
