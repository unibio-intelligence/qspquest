#!/usr/bin/env python3
"""Check generated contracts and Python stub exports against the runtime API."""
import argparse
import ast
import json
from pathlib import Path
from generate_python_types import generate

FORWARDED = {
    'fit_data': ('estimation', 'fit_data'),
    'fit_population': ('population', 'fit_population'),
    'visual_predictive_check': ('diagnostics', 'visual_predictive_check'),
    'profile_likelihood': ('uncertainty', 'profile_likelihood'),
    'bootstrap_population': ('uncertainty', 'bootstrap_population'),
    'compare_regimens': ('exposure', 'compare_regimens'),
    'event_sensitivities': ('derivatives', 'event_sensitivities'),
}


def functions(tree):
    return {node.name: node for node in tree.body if isinstance(node, ast.FunctionDef)}


def exports(tree):
    names = set()
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.ClassDef)):
            names.add(node.name)
        elif isinstance(node, ast.ImportFrom):
            names.update(alias.asname or alias.name for alias in node.names)
        elif isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name):
            names.add(node.target.id)
    return names


def public_parameters(function):
    args = function.args
    names = [node.arg for node in (*args.posonlyargs, *args.args)]
    defaults = {name for name in names[len(names) - len(args.defaults):]} if args.defaults else set()
    keywords = {node.arg for node in args.kwonlyargs if not node.arg.startswith('_')}
    required = {node.arg for node, default in zip(args.kwonlyargs, args.kw_defaults)
                if default is None and not node.arg.startswith('_')}
    return names, defaults, keywords, required


def forwarded_keywords(trees, name):
    module, target = FORWARDED[name]
    function = functions(trees[module])[target]
    keys = public_parameters(function)[2]
    if name == 'bootstrap_population':
        keys |= public_parameters(functions(trees['population'])['fit_population'])[2]
    return keys


def signature(runtime, stub, forwarded=()):
    actual, expected = public_parameters(runtime), public_parameters(stub)
    if actual[:2] != expected[:2] or actual[3] != expected[3]:
        raise ValueError('Positional/default/required signature differs: ' + runtime.name)
    permitted = actual[2] | set(forwarded)
    if permitted != expected[2]:
        raise ValueError('Keyword signature differs: ' + runtime.name + ' ' + str(sorted(permitted ^ expected[2])))
    if stub.args.kwarg is not None:
        raise ValueError('Stable stub keywords must be explicit: ' + runtime.name)


def check(root):
    package = root / 'bindings/python/python/qspquest'
    files = {path.stem: path for path in package.glob('*.py')}
    trees = {name: ast.parse(path.read_text(), feature_version=(3, 10)) for name, path in files.items()}
    stubs = {path.stem: ast.parse(path.read_text(), feature_version=(3, 10)) for path in package.glob('*.pyi')}
    if (package / 'types.py').read_text() != generate(root):
        raise ValueError('Generated Python types differ from the current native schemas')
    counts = {'generated_native_schemas': 16, 'signatures': 0, 'result_methods': 0}
    for module, stub in stubs.items():
        runtime = trees[module]
        runtime_functions, stub_functions = functions(runtime), functions(stub)
        for name, function in stub_functions.items():
            owner, target = module, name
            forwarded = set()
            if name in FORWARDED and module == '__init__':
                owner, target = FORWARDED[name]
                forwarded = forwarded_keywords(trees, name)
            elif name == 'fit_population_dataset':
                forwarded = forwarded_keywords(trees, 'fit_population')
            elif name == 'fit_diagnostics':
                owner = 'diagnostics'
            elif name == 'exposure_summary':
                owner = 'exposure'
            elif name == 'write_analysis_report':
                owner = 'report'
            elif name == 'population_fit_dataset':
                owner = 'dataset'
            elif name == 'individual_fit_dataset':
                owner = 'individual'
            actual = functions(trees[owner]).get(target)
            if actual is None:
                raise ValueError('Stub function missing from runtime: ' + name)
            signature(actual, function, forwarded)
            counts['signatures'] += 1
        runtime_classes = {node.name: node for node in runtime.body if isinstance(node, ast.ClassDef)}
        if module == '__init__':
            dataset = next(node for node in trees['dataset'].body
                           if isinstance(node, ast.ClassDef) and node.name == 'PopulationFitDataset')
            runtime_classes['PopulationFitDataset'] = dataset
            individual = next(node for node in trees['individual'].body
                              if isinstance(node, ast.ClassDef) and node.name == 'IndividualFitDataset')
            runtime_classes['IndividualFitDataset'] = individual
        for declared in (node for node in stub.body if isinstance(node, ast.ClassDef)):
            if declared.name not in runtime_classes:
                raise ValueError('Stub class absent from runtime: ' + declared.name)
            actual = runtime_classes[declared.name]
            if declared.name in ('PopulationFitDataset', 'IndividualFitDataset'):
                def fields(tree):
                    return [node.target.id for node in tree.body
                            if isinstance(node, ast.AnnAssign) and isinstance(node.target, ast.Name)]
                if fields(actual) != fields(declared) or ast.dump(actual.decorator_list[0]) != ast.dump(declared.decorator_list[0]):
                    raise ValueError('Prepared dataset fields or dataclass constructor differ')
            actual_methods = functions(actual)
            for name, function in functions(declared).items():
                if name not in actual_methods:
                    raise ValueError('Stub method absent from runtime: ' + declared.name + '.' + name)
                forwarded = forwarded_keywords(trees, name) if declared.name == 'CompiledModel' and name in FORWARDED else ()
                if declared.name == 'PopulationFitDataset' and name == 'fit':
                    forwarded = forwarded_keywords(trees, 'fit_population')
                    if ast.unparse(function.returns) != 'AnalysisResult':
                        raise ValueError('Runtime helper result is missing its AnalysisResult type: dataset.fit')
                    counts['result_methods'] += 1
                signature(actual_methods[name], function, forwarded)
                counts['signatures'] += 1
                if declared.name in ('CompiledModel', 'CompiledSensitivities'):
                    call = next((node.value for node in actual_methods[name].body if isinstance(node, ast.Return)), None)
                    if isinstance(call, ast.Call) and isinstance(call.func, ast.Name) and call.func.id == 'AnalysisResult':
                        if ast.unparse(function.returns) != 'AnalysisResult':
                            raise ValueError('Runtime helper result is missing its AnalysisResult type: ' + name)
                        counts['result_methods'] += 1
    declared = exports(stubs['__init__'])
    for node in trees['__init__'].body:
        values = None
        if isinstance(node, ast.Assign) and any(isinstance(target, ast.Name) and target.id == '__all__' for target in node.targets):
            values = node.value
        if isinstance(node, ast.AugAssign) and isinstance(node.target, ast.Name) and node.target.id == '__all__':
            values = node.value
        if values is not None:
            names = ast.literal_eval(values)
            missing = set(names) - declared
            if missing:
                raise ValueError('Runtime exports absent from stubs: ' + str(sorted(missing)))
    return {'status': 'matched', **counts}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    try:
        print(json.dumps(check(args.root)))
    except (ValueError, KeyError, OSError) as error:
        parser.exit(1, 'Python contract mismatch: ' + str(error) + '\n')
