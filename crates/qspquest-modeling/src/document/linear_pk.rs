use super::*;
use qspquest_core::expression::{Binary, Expr};
#[derive(Debug)]
pub(super) struct Binding {
    pub(super) amount_unit: String,
    clearance: Program,
    volumes: Vec<Program>,
    exchange: Vec<Program>,
    absorption: Option<Program>,
}
impl Binding {
    pub(super) fn bind(&self, values: &[f64]) -> Result<crate::linear_pk::LinearPk, Error> {
        crate::linear_pk::LinearPk::new(
            self.clearance.evaluate(values)?,
            &self
                .volumes
                .iter()
                .map(|p| p.evaluate(values))
                .collect::<Result<Vec<_>, _>>()?,
            &self
                .exchange
                .iter()
                .map(|p| p.evaluate(values))
                .collect::<Result<Vec<_>, _>>()?,
            self.absorption
                .as_ref()
                .map(|p| p.evaluate(values))
                .transpose()?,
        )
    }
}
impl CompiledDocument {
    pub(super) fn lower_linear_pk(document: &ModelDocument) -> Result<ModelDocument, Error> {
        let pk = document
            .linear_pk
            .as_ref()
            .ok_or_else(|| invalid("missing linear_pk declaration"))?;
        if !document.states.is_empty()
            || !(1..=3).contains(&pk.compartments.len())
            || pk.exchange_clearances.len() + 1 != pk.compartments.len()
        {
            return Err(invalid("linear_pk requires 1..3 compartments, matching exchange clearances and no explicit states"));
        }
        let amount = Unit::parse(&pk.amount_unit)?;
        if !["mg", "mol", "1"]
            .iter()
            .any(|u| amount.conversion_to(Unit::parse(u).unwrap()).is_ok())
        {
            return Err(invalid(
                "linear_pk amount_unit must be mass, amount of substance or dimensionless",
            ));
        }
        fn static_name(
            name: &str,
            document: &ModelDocument,
            visiting: &mut BTreeSet<String>,
            memo: &mut BTreeMap<String, bool>,
        ) -> bool {
            if let Some(value) = memo.get(name) {
                return *value;
            }
            if document.parameters.iter().any(|p| p.name == name) {
                return true;
            }
            if let Some(c) = document.covariates.iter().find(|c| c.name == name) {
                return c.interpolation == Interpolation::Constant;
            }
            if visiting.len() >= 64 || !visiting.insert(name.into()) {
                return false;
            }
            fn static_expr(
                expr: &Expr,
                document: &ModelDocument,
                visiting: &mut BTreeSet<String>,
                memo: &mut BTreeMap<String, bool>,
            ) -> bool {
                if let Expr::Symbol { name } = expr {
                    return static_name(name, document, visiting, memo);
                }
                expr.children()
                    .iter()
                    .all(|e| static_expr(e, document, visiting, memo))
            }
            let valid = document
                .individual
                .iter()
                .find(|i| i.name == name)
                .is_some_and(|i| static_expr(&i.expression, document, visiting, memo));
            visiting.remove(name);
            memo.insert(name.into(), valid);
            valid
        }
        let mut memo = BTreeMap::new();
        for name in std::iter::once(&pk.clearance)
            .chain(pk.compartments.iter().map(|c| &c.volume))
            .chain(&pk.exchange_clearances)
            .chain(pk.depot.iter().map(|d| &d.absorption))
        {
            if !static_name(name, document, &mut BTreeSet::new(), &mut memo) {
                return Err(Error::new(
                    ErrorCode::Unsupported,
                    "linear PK coefficients must depend only on parameters and constant covariates",
                ));
            }
        }
        let mul = |a, b| Expr::binary(Binary::Multiply, a, b);
        let div = |a, b| Expr::binary(Binary::Divide, a, b);
        let add = |a, b| Expr::binary(Binary::Add, a, b);
        let sub = |a, b| Expr::binary(Binary::Subtract, a, b);
        let central = &pk.compartments[0];
        let mut rhs = pk
            .compartments
            .iter()
            .map(|c| Expr::symbol(format!("input_{}", c.state)))
            .collect::<Vec<_>>();
        rhs[0] = sub(
            rhs[0].clone(),
            mul(
                div(Expr::symbol(&pk.clearance), Expr::symbol(&central.volume)),
                Expr::symbol(&central.state),
            ),
        );
        for (i, q) in pk.exchange_clearances.iter().enumerate() {
            let peripheral = &pk.compartments[i + 1];
            let outward = mul(
                div(Expr::symbol(q), Expr::symbol(&central.volume)),
                Expr::symbol(&central.state),
            );
            let inward = mul(
                div(Expr::symbol(q), Expr::symbol(&peripheral.volume)),
                Expr::symbol(&peripheral.state),
            );
            rhs[0] = add(sub(rhs[0].clone(), outward.clone()), inward.clone());
            rhs[i + 1] = sub(add(rhs[i + 1].clone(), outward), inward);
        }
        let state = |name: &str, initial: &Initial, rhs: Expr, dosing: &Option<Dosing>| State {
            name: name.into(),
            unit: pk.amount_unit.clone(),
            initial: initial.clone(),
            rhs,
            dosing: Some(dosing.clone().unwrap_or_else(|| Dosing {
                modes: default_delivery_modes(),
                amount_unit: pk.amount_unit.clone(),
                scale: Expr::number(1.),
                lag: None,
            })),
        };
        let mut lowered = document.clone();
        lowered.linear_pk = None;
        if let Some(depot) = &pk.depot {
            let absorbed = mul(Expr::symbol(&depot.absorption), Expr::symbol(&depot.state));
            rhs[0] = add(rhs[0].clone(), absorbed.clone());
            lowered.states.push(state(
                &depot.state,
                &depot.initial,
                sub(Expr::symbol(format!("input_{}", depot.state)), absorbed),
                &depot.dosing_override,
            ));
        }
        for (c, rhs) in pk.compartments.iter().zip(rhs) {
            lowered
                .states
                .push(state(&c.state, &c.initial, rhs, &c.dosing_override));
        }
        lowered.invariants.push(Invariant::Nonnegative {
            states: lowered.states.iter().map(|s| s.name.clone()).collect(),
        });
        Ok(lowered)
    }
    pub(super) fn compile_linear_pk(document: &ModelDocument) -> Result<Arc<Self>, Error> {
        let lowered = Self::lower_linear_pk(document)?;
        let pk = document
            .linear_pk
            .as_ref()
            .ok_or_else(|| invalid("missing linear_pk declaration"))?;
        let mut compiled = Self::compile(&lowered)?;
        let mut symbols = compiled
            .parameters
            .iter()
            .map(|p| {
                Ok((
                    Symbol {
                        name: p.name.clone(),
                        kind: SymbolKind::Parameter,
                    },
                    Unit::parse(&p.default.unit)?,
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        symbols.extend(
            compiled
                .covariates
                .iter()
                .map(|c| {
                    Ok((
                        Symbol {
                            name: c.name.clone(),
                            kind: SymbolKind::Parameter,
                        },
                        Unit::parse(&c.unit)?,
                    ))
                })
                .collect::<Result<Vec<_>, Error>>()?,
        );
        symbols.extend(compiled.individual.iter().map(|(name, unit, _)| {
            (
                Symbol {
                    name: name.clone(),
                    kind: SymbolKind::Parameter,
                },
                *unit,
            )
        }));
        let time = Unit::parse(&document.time_unit)?;
        let volume = Unit::parse("L")?;
        let program = |name: &str, unit: Unit| {
            Program::compile_with_units(&Expr::symbol(name), &symbols, unit)
        };
        let binding = Binding {
            amount_unit: pk.amount_unit.clone(),
            clearance: program(&pk.clearance, volume.divide(time)?)?,
            volumes: pk
                .compartments
                .iter()
                .map(|c| program(&c.volume, volume))
                .collect::<Result<Vec<_>, _>>()?,
            exchange: pk
                .exchange_clearances
                .iter()
                .map(|c| program(c, volume.divide(time)?))
                .collect::<Result<Vec<_>, _>>()?,
            absorption: pk
                .depot
                .as_ref()
                .map(|d| program(&d.absorption, Unit::ONE.divide(time).unwrap()))
                .transpose()?,
        };
        let inner = Arc::get_mut(&mut compiled).expect("new linear PK model");
        inner.linear_pk = Some(binding);
        inner.model_content_hash = qspquest_core::identity::canonical_hash(
            &serde_json::to_value(document).map_err(|e| invalid(e.to_string()))?,
        )?;
        if document.covariates.iter().all(|c| c.default.is_some()) {
            compiled.bind(&BTreeMap::new())?;
        }
        Ok(compiled)
    }
}
