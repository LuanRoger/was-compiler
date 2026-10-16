# M4 machine-generated comparison tables

Raw command output and environment/source manifests are adjacent. Timings are observations, not speedup claims.

## Unchanged M0 numeric corpus

Pages show initial/final capacity, not live bytes. Source/module identity uses SHA-256.

| Case | Source identical | Module identical | Module bytes M0/M4 | Bump bytes M0/M4 | Pages M0 / M4 |
| --- | --- | --- | ---: | ---: | --- |
| construct_0 | True | True | 776/776 | 1024/1024 | [1, 1] / [1, 1] |
| construct_1 | True | True | 790/790 | 2048/2048 | [1, 1] / [1, 1] |
| construct_7 | True | True | 876/876 | 8192/8192 | [1, 1] / [1, 1] |
| construct_8 | True | True | 890/890 | 9216/9216 | [1, 1] / [1, 1] |
| construct_9 | True | True | 904/904 | 10240/10240 | [1, 1] / [1, 1] |
| page_crossing | True | True | 803/803 | 131072/131072 | [1, 3] / [1, 3] |
| alias_index_replace | True | True | 884/884 | 32/32 | [1, 1] / [1, 1] |
| iteration_live_alias | True | True | 943/943 | 40/40 | [1, 1] / [1, 1] |
| equality_length | True | True | 947/947 | 88/88 | [1, 1] / [1, 1] |
| equality_early | True | True | 1046/1046 | 144/144 | [1, 1] / [1, 1] |
| equality_late | True | True | 1046/1046 | 144/144 | [1, 1] / [1, 1] |
| equality_equal | True | True | 1046/1046 | 144/144 | [1, 1] / [1, 1] |
| string_7_equal | True | True | 786/786 | 0/0 | [1, 1] / [1, 1] |
| string_8_early | True | True | 803/803 | 0/0 | [1, 1] / [1, 1] |
| string_9_late | True | True | 812/812 | 0/0 | [1, 1] / [1, 1] |
| capture_shared | True | True | 946/946 | 48/48 | [1, 1] / [1, 1] |
| capture_iteration | True | True | 929/929 | 8232/8232 | [1, 1] / [1, 1] |

### Guest medians (ns/main call)

| Case | M0 run 1 / 2 | M4 run 1 / 2 |
| --- | ---: | ---: |
| construct_0 | 1950 / 2237.5 | 5225 / 4525 |
| construct_1 | 2300 / 2362.5 | 6012.5 / 6337.5 |
| construct_7 | 3712.5 / 3537.5 | 6637.5 / 10512.5 |
| construct_8 | 4137.5 / 3825 | 9862.5 / 7950 |
| construct_9 | 5325 / 5100 | 10337.5 / 8537.5 |
| page_crossing | 84637.5 / 91662.5 | 176050 / 130950 |
| alias_index_replace | 6425 / 5925 | 11012.5 / 10925 |
| iteration_live_alias | 14012.5 / 11575 | 26687.5 / 26250 |
| equality_length | 2362.5 / 1950 | 4450 / 3862.5 |
| equality_early | 2625 / 2200 | 4975 / 4987.5 |
| equality_late | 4250 / 6100 | 7425 / 8012.5 |
| equality_equal | 4425 / 4000 | 8175 / 7962.5 |
| string_7_equal | 3675 / 3600 | 6212.5 / 7150 |
| string_8_early | 2237.5 / 1962.5 | 4537.5 / 4650 |
| string_9_late | 3750 / 3100 | 7400 / 7400 |
| capture_shared | 3675 / 3375 | 7437.5 / 7187.5 |
| capture_iteration | 13075 / 13900 | 29325 / 28662.5 |

## New matched type-width controls (no historical new-type baseline)

| Case | Source bytes | Module bytes | Bump bytes | Pages initial/final | Guest median ns run 1 / 2 |
| --- | ---: | ---: | ---: | --- | ---: |
| number_construct_0 | 129 | 776 | 1024 | [1, 1] | 6412.5 / 4225 |
| number_construct_1 | 130 | 790 | 2048 | [1, 1] | 7000 / 4787.5 |
| number_construct_2 | 132 | 805 | 3072 | [1, 1] | 7787.5 / 5937.5 |
| number_construct_3 | 134 | 819 | 4096 | [1, 1] | 8100 / 6737.5 |
| number_construct_7 | 142 | 876 | 8192 | [1, 1] | 7087.5 / 6875 |
| number_construct_8 | 144 | 890 | 9216 | [1, 1] | 7062.5 / 9675 |
| number_construct_9 | 146 | 904 | 10240 | [1, 1] | 10700 / 7737.5 |
| number_page_8 | 135 | 890 | 589824 | [1, 10] | 283350 / 311225 |
| number_alias_replace | 188 | 900 | 32 | [1, 1] | 10187.5 / 9800 |
| number_iteration_live | 192 | 965 | 40 | [1, 1] | 30562.5 / 23612.5 |
| number_equality_length | 155 | 947 | 88 | [1, 1] | 5412.5 / 6962.5 |
| number_equality_early | 169 | 1046 | 144 | [1, 1] | 6662.5 / 5525 |
| number_equality_late | 170 | 1046 | 144 | [1, 1] | 9912.5 / 9612.5 |
| number_equality_equal | 169 | 1046 | 144 | [1, 1] | 10350 / 6962.5 |
| number_capture_shared | 230 | 959 | 48 | [1, 1] | 7287.5 / 6800 |
| number_capture_iteration | 174 | 951 | 8232 | [1, 1] | 26300 / 27662.5 |
| bool_construct_0 | 127 | 1051 | 1024 | [1, 1] | 3900 / 4262.5 |
| bool_construct_1 | 131 | 1060 | 2048 | [1, 1] | 4887.5 / 5112.5 |
| bool_construct_2 | 136 | 1069 | 2048 | [1, 1] | 5687.5 / 5575 |
| bool_construct_3 | 141 | 1079 | 2048 | [1, 1] | 6700 / 6587.5 |
| bool_construct_7 | 161 | 1115 | 2048 | [1, 1] | 9600 / 9412.5 |
| bool_construct_8 | 166 | 1124 | 2048 | [1, 1] | 10525 / 10587.5 |
| bool_construct_9 | 171 | 1133 | 3072 | [1, 1] | 13675 / 11612.5 |
| bool_page_8 | 159 | 1124 | 131072 | [1, 3] | 150462.5 / 108375 |
| bool_alias_replace | 201 | 1163 | 16 | [1, 1] | 12912.5 / 11600 |
| bool_iteration_live | 216 | 1209 | 16 | [1, 1] | 34825 / 27775 |
| bool_equality_length | 182 | 1176 | 32 | [1, 1] | 4937.5 / 3775 |
| bool_equality_early | 218 | 1239 | 32 | [1, 1] | 6400 / 5650 |
| bool_equality_late | 219 | 1239 | 32 | [1, 1] | 20075 / 18075 |
| bool_equality_equal | 217 | 1239 | 32 | [1, 1] | 20100 / 18275 |
| bool_capture_shared | 244 | 1216 | 48 | [1, 1] | 8987.5 / 7900 |
| bool_capture_iteration | 191 | 1205 | 8208 | [1, 1] | 36637.5 / 27875 |
| string_construct_0 | 129 | 1053 | 1024 | [1, 1] | 8000 / 5175 |
| string_construct_1 | 140 | 1079 | 2048 | [1, 1] | 10925 / 6162.5 |
| string_construct_2 | 152 | 1088 | 2048 | [1, 1] | 11925 / 7425 |
| string_construct_3 | 164 | 1098 | 3072 | [1, 1] | 16350 / 9150 |
| string_construct_7 | 212 | 1134 | 5120 | [1, 1] | 21250 / 16850 |
| string_construct_8 | 224 | 1143 | 5120 | [1, 1] | 18400 / 18450 |
| string_construct_9 | 236 | 1152 | 6144 | [1, 1] | 23262.5 / 18662.5 |
| string_page_8 | 215 | 1143 | 327680 | [1, 6] | 490012.5 / 462612.5 |
| string_alias_replace | 228 | 1203 | 24 | [1, 1] | 21125 / 20700 |
| string_iteration_live | 262 | 1249 | 24 | [1, 1] | 27162.5 / 26012.5 |
| string_equality_length | 245 | 1195 | 56 | [1, 1] | 4087.5 / 3950 |
| string_equality_early | 329 | 1282 | 80 | [1, 1] | 13387.5 / 12775 |
| string_equality_late | 330 | 1282 | 80 | [1, 1] | 43737.5 / 29862.5 |
| string_equality_equal | 329 | 1258 | 80 | [1, 1] | 29887.5 / 32312.5 |
| string_capture_shared | 270 | 1256 | 48 | [1, 1] | 15487.5 / 15225 |
| string_capture_iteration | 224 | 1245 | 8216 | [1, 1] | 47612.5 / 43225 |

## Unchanged compiler harness (median ns; 100 samples, sample size 1)

Only one historical M0 compiler run exists; no repeated historical compiler comparison is possible.
All nine source/module byte sizes are unchanged. Guest timing is excluded.

| Stage/case | M0 | M4 run 1 | M4 run 2 |
| --- | ---: | ---: | ---: |
| lex/calls_1 | 2099.0 | 3999.0 | 1499.0 |
| lex/calls_16 | 21490.0 | 28290.0 | 28490.0 |
| lex/calls_64 | 61990.0 | 378100.0 | 174300.0 |
| lex/captures_1 | 1899.0 | 2999.0 | 1699.0 |
| lex/captures_16 | 18490.0 | 49340.0 | 14290.0 |
| lex/captures_64 | 54590.0 | 263100.0 | 139500.0 |
| lex/scalar_1 | 799.8 | 1899.0 | 1299.0 |
| lex/scalar_16 | 5199.0 | 7899.0 | 8199.0 |
| lex/scalar_64 | 23590.0 | 29290.0 | 16490.0 |
| parse/calls_1 | 3399.0 | 5899.0 | 3399.0 |
| parse/calls_16 | 59090.0 | 94390.0 | 46140.0 |
| parse/calls_64 | 374500.0 | 674200.0 | 351300.0 |
| parse/captures_1 | 2799.0 | 6399.0 | 3499.0 |
| parse/captures_16 | 39640.0 | 108600.0 | 38290.0 |
| parse/captures_64 | 262900.0 | 558100.0 | 250700.0 |
| parse/scalar_1 | 1399.0 | 3299.0 | 1599.0 |
| parse/scalar_16 | 10690.0 | 31090.0 | 15590.0 |
| parse/scalar_64 | 44040.0 | 124600.0 | 43490.0 |
| semantic/calls_1 | 5099.0 | 8699.0 | 4799.0 |
| semantic/calls_16 | 77190.0 | 127400.0 | 70240.0 |
| semantic/calls_64 | 320700.0 | 466600.0 | 276800.0 |
| semantic/captures_1 | 7899.0 | 12740.0 | 6999.0 |
| semantic/captures_16 | 101800.0 | 227600.0 | 91440.0 |
| semantic/captures_64 | 388800.0 | 701900.0 | 353600.0 |
| semantic/scalar_1 | 1799.0 | 2799.0 | 1699.0 |
| semantic/scalar_16 | 10290.0 | 15990.0 | 14440.0 |
| semantic/scalar_64 | 38140.0 | 57390.0 | 52540.0 |
| source_compile/calls_1 | 23090.0 | 30690.0 | 28390.0 |
| source_compile/calls_16 | 274700.0 | 514600.0 | 299600.0 |
| source_compile/calls_64 | 1282000.0 | 2021999.9999999998 | 1347000.0 |
| source_compile/captures_1 | 40990.0 | 62440.0 | 41090.0 |
| source_compile/captures_16 | 306700.0 | 489200.0 | 325100.0 |
| source_compile/captures_64 | 1225000.0 | 1853000.0 | 1220000.0 |
| source_compile/scalar_1 | 7199.0 | 17090.0 | 8148.999999999999 |
| source_compile/scalar_16 | 44290.0 | 84890.0 | 44190.0 |
| source_compile/scalar_64 | 173300.0 | 364400.0 | 159000.0 |
| typed_compile/calls_1 | 6849.0 | 16890.0 | 7749.0 |
| typed_compile/calls_16 | 78490.0 | 174300.0 | 80490.0 |
| typed_compile/calls_64 | 279200.0 | 504600.0 | 335600.0 |
| typed_compile/captures_1 | 18790.0 | 35790.0 | 22540.0 |
| typed_compile/captures_16 | 105700.0 | 192200.0 | 156100.0 |
| typed_compile/captures_64 | 411900.0 | 766800.0 | 444800.0 |
| typed_compile/scalar_1 | 3199.0 | 4899.0 | 3499.0 |
| typed_compile/scalar_16 | 11990.0 | 29540.0 | 13390.0 |
| typed_compile/scalar_64 | 38490.0 | 65640.0 | 41690.0 |
