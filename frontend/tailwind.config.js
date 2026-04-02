/** @type {import('tailwindcss').Config} */
export default {
  content: ['./index.html', './src/**/*.{vue,js,ts}'],
  darkMode: 'class',
  theme: {
    extend: {
      colors: {
        primary: '#24A1DE',
        'primary-container': '#1a7aaa',
        'on-primary': '#ffffff',
        'surface': '#f8fafc',
        'surface-container': '#f1f5f9',
        'surface-container-low': '#e8eef4',
        'surface-container-high': '#e2e8f0',
        'surface-container-highest': '#cbd5e1',
        'surface-container-lowest': '#ffffff',
        'on-surface': '#0f172a',
        'on-surface-variant': '#475569',
        'outline-variant': '#94a3b8',
        'error': '#ef4444',
        'on-error': '#ffffff',
      }
    }
  },
  plugins: []
}
