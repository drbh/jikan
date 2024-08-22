# Contributing to Jikan

We're excited that you're interested in contributing to Jikan! This document outlines the process for contributing to the project and helps ensure a smooth collaboration experience.

## Getting Started

1. Fork the repository on GitHub.
2. Clone your fork locally:
   ```
   git clone https://github.com/your-username/jikan.git
   ```
3. Create a new branch for your feature or bug fix:
   ```
   git checkout -b feature/your-feature-name
   ```

## Making Changes

1. Make your changes in your feature branch.
2. Add or update tests as necessary.
3. Ensure all tests pass by running:
   ```
   cargo test
   ```
4. Update documentation if you've made changes to the API or added new features.

## Submitting Changes

1. Commit your changes with a clear and descriptive commit message:
   ```
   git commit -am "Add a brief description of your changes"
   ```
2. Push your changes to your fork on GitHub:
   ```
   git push origin feature/your-feature-name
   ```
3. Create a pull request from your fork to the main Jikan repository.

## Pull Request Guidelines

- Provide a clear description of the problem you're solving or the feature you're adding.
- Include any relevant issue numbers in the PR description.
- Ensure your code follows the project's coding style and conventions.
- Make sure all tests pass and add new tests for new functionality.

## Code Style

- Follow Rust's official style guide.
- Use `rustfmt` to format your code before submitting.
- Run `clippy` and address any warnings:
   ```
   cargo clippy
   ```

## Reporting Bugs

- Use the GitHub issue tracker to report bugs.
- Describe the bug in detail, including steps to reproduce.
- Include the version of Jikan you're using and your operating system.

## Suggesting Enhancements

- Use the GitHub issue tracker to suggest enhancements.
- Clearly describe the feature and its potential benefits.
- If possible, provide examples of how the feature would be used.

Thank you for contributing to Jikan! Your efforts help make the project better for everyone.
