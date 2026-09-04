use jcode_render_core::normalize_latex_math;

#[test]
fn parses_the_exact_multiline_equation_response_from_the_tui() {
    let response = concat!(
        "\\[\n\\boxed{\ne^{i\\pi}+1=0\n}\n\\]\n\n",
        "\\[\n\\int_{-\\infty}^{\\infty} e^{-x^2}\\,dx=\\sqrt{\\pi}\n\\]\n\n",
        "\\[\nx=\\frac{-b\\pm\\sqrt{b^2-4ac}}{2a}\n\\]\n\n",
        "\\[\n\\nabla\\cdot\\mathbf{E}=\\frac{\\rho}{\\varepsilon_0}\n\\]\n\n",
        "\\[\n\\frac{\\partial \\psi}{\\partial t}\n=\n",
        "\\alpha\\frac{\\partial^2\\psi}{\\partial x^2}\n\\]",
    );

    // Five `\[...\]` display blocks must normalize to five `$$...$$` pairs.
    let normalized = normalize_latex_math(response);
    assert_eq!(
        normalized.matches("$$").count(),
        10,
        "normalized={normalized:?}"
    );
}

#[test]
fn every_streaming_prefix_is_deterministic_and_the_complete_response_is_math() {
    let equation = concat!(
        "Result:\n\n\\[\n",
        "\\frac{\\partial \\psi}{\\partial t}\n=\n",
        "\\alpha\\frac{\\partial^2\\psi}{\\partial x^2}\n\\]",
    );

    // Streaming reveals the response one char at a time, so normalization of
    // every prefix must be deterministic and idempotent — a prefix that
    // normalized differently on a later frame would make the rendered math
    // flicker or duplicate.
    for end in equation
        .char_indices()
        .map(|(index, _)| index)
        .chain(std::iter::once(equation.len()))
    {
        let prefix = &equation[..end];
        let first = normalize_latex_math(prefix);
        let second = normalize_latex_math(prefix);
        assert_eq!(
            first, second,
            "nondeterministic prefix ending at byte {end}"
        );
        assert_eq!(
            normalize_latex_math(&first),
            first,
            "non-idempotent prefix ending at byte {end}"
        );
    }

    assert_eq!(normalize_latex_math(equation).matches("$$").count(), 2);
}
