# Fit a fraction with a delayed input

This example estimates a fraction from synthetic concentrations after a delayed
systemic input. It uses a fixed inverse-Gaussian input shape, linear elimination
and independent additive Gaussian observation error. It does not estimate
population variation or establish biological validity.

The complete [model](../examples/inverse-gaussian.qsp) and
[fit request](../examples/inverse-gaussian-fit.json) are included.

You need the source checkout and its pinned Rust toolchain. From the checkout
root, build the CLI and run the complete supplied example:

```sh
cargo build --locked --release -p qspquest-cli
target/release/qspquest fit docs/examples/inverse-gaussian.qsp docs/examples/inverse-gaussian-fit.json
```

The result should have `status: "converged"` and a fitted `f` within `0.00001`
of `0.7`. The example fixes the administered amount, volume, elimination rate,
input mean and shape; changing those quantities changes the estimand.
The observations are synthetic and their additive standard deviation is
`0.1 mg/L`.

`inverse_gaussian_density(time, mean, shape)` takes three quantities with the
same dimension. Its result has the inverse dimension. For positive time,
positive mean and positive shape its value is

```text
sqrt(shape / (2*pi*time^3))
    * exp(-shape*(time-mean)^2 / (2*mean^2*time)).
```

Its integral over positive time is one. At nonpositive time the value is
exactly zero, including the zero-time limit of its clock and parameter
derivatives. Mean and shape must remain positive even when time is zero.
The runtime evaluates the positive-time density in logarithmic form to avoid
intermediate overflow. Invalid arguments and nonrepresentable results fail
with a domain error.

Multiply the density by an amount to obtain an input rate. In the supplied
model, an amount in `mg` and times in `h` produce a rate in `mg/h`.
The shape parameter has units of time; it is not a dimensionless coefficient
of variation. For a squared coefficient of variation `cv2`, use
`shape = mean / cv2`.

For scalar fitting, keep the fitted parameter inside the request's bounds and
make sure it is marked `estimated`. Observation row numbers refer to the
ordered requested output times, including the initial zero-time row. The
generic restriction on dynamic conditional expressions still applies; use the
qualified density directly for this input.
