/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{ts,tsx}'],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        ink: {
          900: '#000000',
          800: '#0a0a0a',
          700: '#141414',
          600: '#1f1f1f',
          500: '#2a2a2a',
          400: '#3d3d3d',
          300: '#737373',
          200: '#a3a3a3',
          100: '#e5e5e5',
        },
        accent: '#e5e5e5',
        positive: '#9ca3af',
        negative: '#a1a1aa',
      },
      fontFamily: {
        sans: ['system-ui', '-apple-system', 'Segoe UI', 'Roboto', 'sans-serif'],
        mono: ['ui-monospace', 'SFMono-Regular', 'Menlo', 'Consolas', 'monospace'],
      },
    },
  },
  plugins: [],
};
