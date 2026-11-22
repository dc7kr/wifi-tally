module.exports = {
  transform: { '^.+\\.[tj]s$': 'ts-jest' },
  testEnvironment: 'node',
  roots: ['<rootDir>/src'],
  testMatch: ['**/*.spec.ts', '**/*.spec.js'],
  setupFilesAfterEnv: ['jest-extended'],
  globals: {
    'ts-jest': { tsconfig: 'tsconfig.json', diagnostics: false },
  },
}
