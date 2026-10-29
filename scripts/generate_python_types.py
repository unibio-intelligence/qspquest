#!/usr/bin/env python3
"""Generate dictionary-compatible Python contracts from the native JSON schemas."""
import argparse
import hashlib
import json
import keyword
from pathlib import Path

SCHEMAS = (
    'run', 'result', 'sensitivity-request', 'sensitivity-result',
    'scalar-fit-request', 'scalar-fit-result', 'fit-request', 'fit-result',
    'morris-request', 'morris-result', 'scan-request', 'scan-result',
    'steady-state-request', 'steady-state-result',
    'cycle-iteration-request', 'cycle-iteration-result',
    'gamma-poisson-fit', 'discrete-fit-result',
    'probit-normal-binomial-fit', 'probit-normal-binomial-result',
)
PRIMITIVES = {'string': 'str', 'number': 'float', 'integer': 'int',
              'boolean': 'bool', 'null': 'None'}


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'))


def closed(schema, node, seen=()):
    """Compare referenced structure, so same-named enums cannot alias by accident."""
    if isinstance(node, list):
        return [closed(schema, value, seen) for value in node]
    if not isinstance(node, dict):
        return node
    if '$ref' in node:
        ref = node['$ref']
        if not ref.startswith('#/definitions/'):
            raise ValueError('Unsupported external schema reference: ' + ref)
        if ref in seen:
            return {'recursive_reference': ref.rsplit('/', 1)[1]}
        return closed(schema, schema['definitions'][ref.rsplit('/', 1)[1]], (*seen, ref))
    return {key: closed(schema, value, seen) for key, value in node.items()
            if key not in ('description', 'title', 'default', 'format', '$schema',
                           'x-qspquest-spec-version', 'x-qspquest-validation')}


class Generator:
    def __init__(self, schemas):
        self.schemas = schemas
        self.names = {}
        self.nodes = {}
        self.aliases = {}
        self.classes = {}
        variants = {}
        for schema in schemas:
            for name, value in schema.get('definitions', {}).items():
                variants.setdefault(name, set()).add(canonical(closed(schema, value)))
        roots = {schema['title'] for schema in schemas}
        for schema in schemas:
            root = schema['title']
            for name, value in schema.get('definitions', {}).items():
                selected = ('Regimen' if name == 'Request' else name)
                if len(variants[name]) > 1 or selected in roots:
                    selected = root + name
                key = root, name
                self.names[key] = selected
                if selected in self.nodes and canonical(closed(*self.nodes[selected])) != canonical(closed(schema, value)):
                    raise ValueError('Generated type-name collision: ' + selected)
                self.nodes[selected] = schema, value
            self.nodes[root] = schema, schema

    def expression(self, schema, value, name, *, quoted=False):
        if '$ref' in value:
            ref = value['$ref']
            if not ref.startswith('#/definitions/'):
                raise ValueError('Unsupported reference: ' + ref)
            selected = self.names[schema['title'], ref.rsplit('/', 1)[1]]
            return repr(selected) if quoted else selected
        if 'allOf' in value:
            if len(value['allOf']) != 1:
                raise ValueError('Unsupported multi-schema intersection')
            return self.expression(schema, value['allOf'][0], name, quoted=quoted)
        for union in ('oneOf', 'anyOf'):
            if union in value:
                parts = [self.expression(schema, item, name + str(index + 1), quoted=quoted)
                         for index, item in enumerate(value[union])]
                return 'Union[' + ', '.join(dict.fromkeys(parts)) + ']'
        if 'enum' in value:
            return 'Literal[' + ', '.join(repr(item) for item in value['enum']) + ']'
        kind = value.get('type')
        if isinstance(kind, list):
            return 'Union[' + ', '.join(self.expression(schema, {**value, 'type': item}, name, quoted=quoted)
                                       for item in kind) + ']'
        if kind in PRIMITIVES:
            return PRIMITIVES[kind]
        if kind == 'array':
            items = value.get('items')
            if not isinstance(items, dict):
                raise ValueError('Array contract requires one item schema')
            return 'list[' + self.expression(schema, items, name + 'Item', quoted=quoted) + ']'
        if kind == 'object':
            if 'properties' in value:
                if value.get('additionalProperties') is not False:
                    raise ValueError('Typed records require closed properties: ' + name)
                self.record(schema, value, name)
                return repr(name) if quoted else name
            extra = value.get('additionalProperties', {})
            if extra is False:
                raise ValueError('Unsupported empty closed record')
            return 'dict[str, ' + self.expression(schema, extra if isinstance(extra, dict) else {}, name + 'Value', quoted=quoted) + ']'
        if not value:
            return 'object'
        raise ValueError('Unsupported type schema: ' + canonical(value))

    def record(self, schema, value, name):
        properties = value['properties']
        required = set(value.get('required', ()))
        if required - properties.keys():
            raise ValueError('Required field absent from properties: ' + name)
        fields = {}
        for field, contract in sorted(properties.items()):
            if not field.isidentifier() or keyword.iskeyword(field):
                raise ValueError('Unsupported Python field name: ' + field)
            fields[field] = self.expression(schema, contract, name + ''.join(part.title() for part in field.split('_')))
        existing = self.classes.get(name)
        if existing is not None and existing != (fields, required):
            raise ValueError('Conflicting inline record: ' + name)
        self.classes[name] = fields, required

    def render(self):
        for name, (schema, value) in self.nodes.items():
            if value.get('type') == 'object' and 'properties' in value:
                self.record(schema, value, name)
            else:
                self.aliases[name] = self.expression(schema, value, name, quoted=True)
        lines = ['"""Structural native contracts; binding validates units, ranges and support."""',
                 'from __future__ import annotations',
                 'from typing import Literal, TypedDict, Union', '',
                 '# Regenerate with: python scripts/generate_python_types.py', '']
        for name, expression in sorted(self.aliases.items()):
            lines.append(name + ' = ' + expression)
        for name, (fields, required) in sorted(self.classes.items()):
            mandatory = {field: annotation for field, annotation in fields.items() if field in required}
            optional = {field: annotation for field, annotation in fields.items() if field not in required}
            if mandatory and optional:
                lines += ['', 'class _' + name + 'Required(TypedDict):']
                lines += ['    ' + field + ': ' + annotation for field, annotation in mandatory.items()]
                lines += ['', 'class ' + name + '(_' + name + 'Required, total=False):']
                lines += ['    ' + field + ': ' + annotation for field, annotation in optional.items()]
            else:
                lines += ['', 'class ' + name + '(TypedDict' + (', total=False' if optional else '') + '):']
                lines += ['    ' + field + ': ' + annotation for field, annotation in fields.items()] or ['    pass']
        return '\n'.join(lines) + '\n'


def generate(root):
    schemas = [json.loads((root / 'spec/schema' / (name + '.schema.json')).read_text()) for name in SCHEMAS]
    return Generator(schemas).render()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--check', action='store_true', help='Fail if the generated contracts differ from the current schemas')
    args = parser.parse_args()
    output = args.root / 'bindings/python/python/qspquest/types.py'
    text = generate(args.root)
    if args.check:
        if not output.is_file() or output.read_text() != text:
            parser.exit(1, 'Python contracts differ from native schemas; regenerate with scripts/generate_python_types.py\n')
    else:
        output.write_text(text)
    print(json.dumps({'status': 'matched' if args.check else 'generated',
                      'sha256': hashlib.sha256(text.encode()).hexdigest(), 'schemas': len(SCHEMAS)}))


if __name__ == '__main__':
    main()
