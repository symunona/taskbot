// Production bundle: mangle only, no source maps.
// With webpack's defaults (terser compress passes + source maps) minifying the Compose bundle
// did not finish in 35 minutes on the build machine; without source maps, not in 17 either.
if (config.mode === 'production') {
    const TerserPlugin = require('terser-webpack-plugin');
    config.devtool = false;
    config.module.rules = config.module.rules.filter(
        (rule) => !(rule.use && rule.use.includes && rule.use.includes('source-map-loader'))
    );
    config.optimization = config.optimization || {};
    config.optimization.minimizer = [
        new TerserPlugin({ extractComments: false, terserOptions: { compress: false, mangle: true } }),
    ];
}
