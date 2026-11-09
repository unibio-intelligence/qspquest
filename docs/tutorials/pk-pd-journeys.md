# PK/PD tutorials

These tutorials follow the usual order of a pharmacokinetic (PK) and
pharmacodynamic (PD) analysis: model definition, dosing and simulation, model
fitting, population analysis and model evaluation. Each tutorial uses synthetic
data, provides a script that can be run as written, and lists the expected
results. Units follow common pharmacometric practice: clearance (`CL`) in L/h,
volume (`V`) in L, absorption rate constant (`KA`) in h⁻¹ and concentrations in
mg/L.

| Step | Tutorial | Content |
| --- | --- | --- |
| 1. Single dose | [Single-dose pharmacokinetics](single-dose-pk.md) | IV bolus, IV infusion and oral profiles compared with analytical solutions; apparent clearance (CL/F); effect of the sampling schedule on Tmax and AUC; flip-flop kinetics |
| 2. Multiple doses | [Multiple-dose pharmacokinetics](multiple-dose-pk.md) | Accumulation and time to steady state; pre-dose and post-dose sampling; steady state for unequal dosing intervals; loading dose; comparison of schedules in the same virtual subjects |
| 3. Noncompartmental analysis | [Noncompartmental analysis of single-dose profiles](noncompartmental-analysis.md) | Terminal rate constant (λz) and half-life, AUC0–∞ and the percentage extrapolated, CL/F and Vz/F from observed concentrations; pre-dose and BLQ rules; comparison with closed-form values |
| 4. Individual fitting | [Individual fitting of synthetic oral PK data](oral-pk-analysis.md) | Estimation of `cl`, `vc` and `ka` for 12 subjects; residual diagnostics; HTML report |
| 5. Published data | [Fitting published theophylline concentrations](published-theophylline.md) | Fit of one subject from the `theo_sd` dataset with BDF and a Gaussian error model |
| 6. Population fitting | [Population pharmacokinetics of an oral drug](population-pk.md) · [Python guide: population data](../PYTHON-USER-GUIDE.md#fit-population-data) · [SAEM](../SAEM.md) · [Log-scale Wald intervals for SAEM](population-log-wald.md) | Estimation of typical values, interindividual variability on `CL`, `V` and `KA`, a body-weight effect on clearance and a combined residual error from NONMEM-style records; goodness-of-fit plots, prediction-corrected visual predictive check and bootstrap [8–11] |
| 7. Model evaluation | [Visual predictive checks](parallel-predictive-checks.md) · [profile likelihood and bootstrap](../ANALYSIS.md) | Visual predictive checks, likelihood profiles and bootstrap confidence intervals |
| 8. Pharmacodynamics | [Direct, effect-compartment and indirect response models](pk-pd-response-models.md) · [paired-endpoint fitting](paired-endpoint-fitting.md) | Direct sigmoid Emax, effect-compartment and indirect-response models linked to an oral PK model [5–7]; joint fit of concentration and response data |

## Data records

Population fitting reads NONMEM-style records with the columns `ID`, `TIME`,
`EVID`, `DV`, `MDV`, `AMT`, `CMT`, `RATE`, `ADDL`, `II` and `SS`. Each dosing
compartment (`CMT`) and each observation type must be mapped to a model state
or output; no mapping is inferred. Records are rejected if `SS` is nonzero,
`EVID` or `MDV` is other than 0 or 1, `RATE` is negative, `ADDL` is negative or
not an integer, or a column is not one of those listed. The [Python
guide](../PYTHON-USER-GUIDE.md#fit-population-data) gives the complete rules.

## Exposure metrics and NCA

`exposure_summary` computes AUC, Cmax, Tmax, Cmin and the time above a threshold
from a simulated profile. It uses the linear trapezoidal rule between the
sampled times and does not extrapolate, so it is not a noncompartmental
analysis (NCA). NCA of observed concentrations is performed by
`noncompartmental_analysis`, which estimates λz, the terminal half-life,
`AUC0–∞`, the percentage of AUC extrapolated, `CL/F` and `Vz/F`; its rules are
described in the [NCA tutorial](noncompartmental-analysis.md). Standard NCA
methods are described by Gabrielsson and Weiner [1] and in regulatory guidance
on bioavailability and bioequivalence [2, 3]. An independent open-source
implementation is available in the PKNCA package [4].

## Python and R interfaces

All the steps above are available in Python. The R interface supports
simulation (including repeated doses and pre-dose and post-dose sampling),
individual and population fitting, NONMEM-style records, `exposure_summary`,
`compare_regimens`, visual predictive checks and bootstrap; see the [R
guide](../R-USER-GUIDE.md). It does not yet provide periodic steady-state
calculation or noncompartmental analysis.

## References

1. Gabrielsson J, Weiner D. Non-compartmental analysis. *Methods Mol Biol*.
   2012;929:377–389. [doi:10.1007/978-1-62703-050-2_16](https://doi.org/10.1007/978-1-62703-050-2_16)
2. US Food and Drug Administration. *Bioavailability Studies Submitted in NDAs
   or INDs — General Considerations*. Guidance for Industry; 2022.
3. International Council for Harmonisation. *M13A: Bioequivalence for
   Immediate-Release Solid Oral Dosage Forms*. Step 4; 2024.
4. Denney B, Buckeridge C, Rodriguez GJ. PKNCA: Perform Pharmacokinetic
   Non-Compartmental Analysis. R package.
   [doi:10.32614/CRAN.package.PKNCA](https://doi.org/10.32614/CRAN.package.PKNCA)
5. Holford NHG, Sheiner LB. Understanding the dose-effect relationship:
   clinical application of pharmacokinetic-pharmacodynamic models.
   *Clin Pharmacokinet*. 1981;6(6):429–453.
   [doi:10.2165/00003088-198106060-00002](https://doi.org/10.2165/00003088-198106060-00002)
6. Sheiner LB, Stanski DR, Vozeh S, Miller RD, Ham J. Simultaneous modeling of
   pharmacokinetics and pharmacodynamics: application to d-tubocurarine.
   *Clin Pharmacol Ther*. 1979;25(3):358–371.
   [doi:10.1002/cpt1979253358](https://doi.org/10.1002/cpt1979253358)
7. Dayneka NL, Garg V, Jusko WJ. Comparison of four basic models of indirect
   pharmacodynamic responses. *J Pharmacokinet Biopharm*. 1993;21(4):457–478.
   [doi:10.1007/BF01061691](https://doi.org/10.1007/BF01061691)
8. Mould DR, Upton RN. Basic concepts in population modeling, simulation, and
   model-based drug development. *CPT Pharmacometrics Syst Pharmacol*.
   2012;1:e6. [doi:10.1038/psp.2012.4](https://doi.org/10.1038/psp.2012.4)
9. Mould DR, Upton RN. Basic concepts in population modeling, simulation, and
   model-based drug development—part 2: introduction to pharmacokinetic
   modeling methods. *CPT Pharmacometrics Syst Pharmacol*. 2013;2:e38.
   [doi:10.1038/psp.2013.14](https://doi.org/10.1038/psp.2013.14)
10. Karlsson MO, Savic RM. Diagnosing model diagnostics. *Clin Pharmacol Ther*.
    2007;82(1):17–20. [doi:10.1038/sj.clpt.6100241](https://doi.org/10.1038/sj.clpt.6100241)
11. Bergstrand M, Hooker AC, Wallin JE, Karlsson MO. Prediction-corrected visual
    predictive checks for diagnosing nonlinear mixed-effects models. *AAPS J*.
    2011;13(2):143–151. [doi:10.1208/s12248-011-9255-z](https://doi.org/10.1208/s12248-011-9255-z)
