import js from "@eslint/js";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import globals from "globals";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist"] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2020,
      globals: globals.browser,
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      // Just the long-established pair (hook-call-order safety,
      // useEffect/useMemo/useCallback dependency-array correctness), not
      // `react-hooks`' newer "recommended" preset, that one also pulls in
      // its React-Compiler-oriented rules (e.g. `set-state-in-effect`,
      // `refs`), which flag a lot of normal, correct code in a codebase
      // that isn't opted into the React Compiler and wasn't written
      // against those rules.
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],
      // The codebase leans on `_`-prefixed params/catch bindings to mark
      // something intentionally unused rather than deleting the binding.
      "@typescript-eslint/no-unused-vars": [
        "warn",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_", caughtErrorsIgnorePattern: "^_" },
      ],
    },
  },
);
