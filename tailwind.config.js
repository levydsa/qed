/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [ 'templates/*.html', "content/*.djot" ],
  darkMode: ['selector', '[data-theme="dark"]'],
  theme: {
    extend: {},
  },
  plugins: [
    require('@tailwindcss/typography'),
  ],
}

