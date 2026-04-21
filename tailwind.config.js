/** @type {import('tailwindcss').Config} */
export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    extend: {
      colors: {
        surface: "#1a1a1a",
        "surface-light": "#2a2a2a",
        text: "#e5e5e5",
        "text-dim": "#9ca3af",
        border: "#333333",
        accent: "#22c55e",
      },
    },
  },
  plugins: [],
};
