# Planr

A project planning and management tool for software development projects.

## Overview

Planr is an AI-powered planning tool that helps teams manage product requirements, create user stories, and track development progress through a structured workflow.

## Project Structure

- `.planr/` - Core directory containing project planning artifacts
  - `stories/` - User stories and requirements 
  - `roadmap.json` - Project roadmap defining tasks and milestones
  - `prd.md` - Product Requirements Document

## Modes

Planr supports different operational modes:

- **Planning Mode** - Create and manage stories, specifications, and project plans
  - `init` - Initialize project with stories and roadmap from PRD
  - `add` - Add new stories to the project

- **Implementation Mode** - Execute on the planned stories using the defined roadmap

## Getting Started

1. Set up your project structure with the `.planr` directory
2. Create your Product Requirements Document in `.planr/prd.md`
3. Use Planning Mode to generate stories and roadmap
4. Implement stories following the roadmap

## Usage

Follow the structured workflow:

1. Define product requirements in `prd.md`
2. Generate stories and roadmap
3. Track progress by updating story status and roadmap
