"""Run the reader scenario from its cached module."""

import importlib

# The import runs the scenario. Python caches the bytecode of an imported module only.
_ = importlib.import_module('reader_scenario_cases')
