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
        // 浅色模式（默认）
        surface: '#f8fafc',
        'surface-container': '#f1f5f9',
        'surface-container-low': '#e8eef4',
        'surface-container-high': '#e2e8f0',
        'surface-container-highest': '#cbd5e1',
        'surface-container-lowest': '#ffffff',
        'on-surface': '#0f172a',
        'on-surface-variant': '#475569',
        'outline-variant': '#94a3b8',
        error: '#ef4444',
        'on-error': '#ffffff',
      },
      // 暗黑模式颜色覆盖
      darkColors: {
        surface: '#0f172a',
        'surface-container': '#1e293b',
        'surface-container-low': '#253349',
        'surface-container-high': '#334155',
        'surface-container-highest': '#475569',
        'surface-container-lowest': '#0b1120',
        'on-surface': '#e2e8f0',
        'on-surface-variant': '#94a3b8',
        'outline-variant': '#334155',
        error: '#fca5a5',
        'on-error': '#0f172a',
      }
    }
  },
  plugins: []
}
