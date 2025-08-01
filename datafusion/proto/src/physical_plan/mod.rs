// Licensed to the Apache Software Foundation (ASF) under one
// or more contributor license agreements.  See the NOTICE file
// distributed with this work for additional information
// regarding copyright ownership.  The ASF licenses this file
// to you under the Apache License, Version 2.0 (the
// "License"); you may not use this file except in compliance
// with the License.  You may obtain a copy of the License at
//
//   http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing,
// software distributed under the License is distributed on an
// "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.  See the License for the
// specific language governing permissions and limitations
// under the License.

use std::fmt::Debug;
use std::sync::Arc;

use self::from_proto::parse_protobuf_partitioning;
use self::to_proto::{serialize_partitioning, serialize_physical_expr};
use crate::common::{byte_to_string, str_to_byte};
use crate::physical_plan::from_proto::{
    parse_physical_expr, parse_physical_sort_expr, parse_physical_sort_exprs,
    parse_physical_window_expr, parse_protobuf_file_scan_config,
    parse_protobuf_file_scan_schema,
};
use crate::physical_plan::to_proto::{
    serialize_file_scan_config, serialize_maybe_filter, serialize_physical_aggr_expr,
    serialize_physical_window_expr,
};
use crate::protobuf::physical_aggregate_expr_node::AggregateFunction;
use crate::protobuf::physical_expr_node::ExprType;
use crate::protobuf::physical_plan_node::PhysicalPlanType;
use crate::protobuf::{
    self, proto_error, window_agg_exec_node, ListUnnest as ProtoListUnnest,
};
use crate::{convert_required, into_required};

use datafusion::arrow::compute::SortOptions;
use datafusion::arrow::datatypes::{Schema, SchemaRef};
use datafusion::datasource::file_format::csv::CsvSink;
use datafusion::datasource::file_format::file_compression_type::FileCompressionType;
use datafusion::datasource::file_format::json::JsonSink;
#[cfg(feature = "parquet")]
use datafusion::datasource::file_format::parquet::ParquetSink;
#[cfg(feature = "avro")]
use datafusion::datasource::physical_plan::AvroSource;
#[cfg(feature = "parquet")]
use datafusion::datasource::physical_plan::ParquetSource;
use datafusion::datasource::physical_plan::{
    CsvSource, FileScanConfig, FileScanConfigBuilder, JsonSource,
};
use datafusion::datasource::sink::DataSinkExec;
use datafusion::datasource::source::DataSourceExec;
use datafusion::execution::runtime_env::RuntimeEnv;
use datafusion::execution::FunctionRegistry;
use datafusion::physical_expr::aggregate::AggregateExprBuilder;
use datafusion::physical_expr::aggregate::AggregateFunctionExpr;
use datafusion::physical_expr::{LexOrdering, LexRequirement, PhysicalExprRef};
use datafusion::physical_plan::aggregates::AggregateMode;
use datafusion::physical_plan::aggregates::{AggregateExec, PhysicalGroupBy};
use datafusion::physical_plan::analyze::AnalyzeExec;
use datafusion::physical_plan::coalesce_batches::CoalesceBatchesExec;
use datafusion::physical_plan::coalesce_partitions::CoalescePartitionsExec;
use datafusion::physical_plan::coop::CooperativeExec;
use datafusion::physical_plan::empty::EmptyExec;
use datafusion::physical_plan::explain::ExplainExec;
use datafusion::physical_plan::expressions::PhysicalSortExpr;
use datafusion::physical_plan::filter::FilterExec;
use datafusion::physical_plan::joins::utils::{ColumnIndex, JoinFilter};
use datafusion::physical_plan::joins::{
    CrossJoinExec, NestedLoopJoinExec, StreamJoinPartitionMode, SymmetricHashJoinExec,
};
use datafusion::physical_plan::joins::{HashJoinExec, PartitionMode};
use datafusion::physical_plan::limit::{GlobalLimitExec, LocalLimitExec};
use datafusion::physical_plan::placeholder_row::PlaceholderRowExec;
use datafusion::physical_plan::projection::ProjectionExec;
use datafusion::physical_plan::repartition::RepartitionExec;
use datafusion::physical_plan::sorts::sort::SortExec;
use datafusion::physical_plan::sorts::sort_preserving_merge::SortPreservingMergeExec;
use datafusion::physical_plan::union::{InterleaveExec, UnionExec};
use datafusion::physical_plan::unnest::{ListUnnest, UnnestExec};
use datafusion::physical_plan::windows::{BoundedWindowAggExec, WindowAggExec};
use datafusion::physical_plan::{
    ExecutionPlan, InputOrderMode, PhysicalExpr, WindowExpr,
};
use datafusion_common::config::TableParquetOptions;
use datafusion_common::{internal_err, not_impl_err, DataFusionError, Result};
use datafusion_expr::{AggregateUDF, ScalarUDF, WindowUDF};

use prost::bytes::BufMut;
use prost::Message;

pub mod from_proto;
pub mod to_proto;

impl AsExecutionPlan for protobuf::PhysicalPlanNode {
    fn try_decode(buf: &[u8]) -> Result<Self>
    where
        Self: Sized,
    {
        protobuf::PhysicalPlanNode::decode(buf).map_err(|e| {
            DataFusionError::Internal(format!("failed to decode physical plan: {e:?}"))
        })
    }

    fn try_encode<B>(&self, buf: &mut B) -> Result<()>
    where
        B: BufMut,
        Self: Sized,
    {
        self.encode(buf).map_err(|e| {
            DataFusionError::Internal(format!("failed to encode physical plan: {e:?}"))
        })
    }

    fn try_into_physical_plan(
        &self,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let plan = self.physical_plan_type.as_ref().ok_or_else(|| {
            proto_error(format!(
                "physical_plan::from_proto() Unsupported physical plan '{self:?}'"
            ))
        })?;
        match plan {
            PhysicalPlanType::Explain(explain) => self.try_into_explain_physical_plan(
                explain,
                registry,
                runtime,
                extension_codec,
            ),
            PhysicalPlanType::Projection(projection) => self
                .try_into_projection_physical_plan(
                    projection,
                    registry,
                    runtime,
                    extension_codec,
                ),
            PhysicalPlanType::Filter(filter) => self.try_into_filter_physical_plan(
                filter,
                registry,
                runtime,
                extension_codec,
            ),
            PhysicalPlanType::CsvScan(scan) => self.try_into_csv_scan_physical_plan(
                scan,
                registry,
                runtime,
                extension_codec,
            ),
            PhysicalPlanType::JsonScan(scan) => self.try_into_json_scan_physical_plan(
                scan,
                registry,
                runtime,
                extension_codec,
            ),
            #[cfg_attr(not(feature = "parquet"), allow(unused_variables))]
            PhysicalPlanType::ParquetScan(scan) => self
                .try_into_parquet_scan_physical_plan(
                    scan,
                    registry,
                    runtime,
                    extension_codec,
                ),
            #[cfg_attr(not(feature = "avro"), allow(unused_variables))]
            PhysicalPlanType::AvroScan(scan) => self.try_into_avro_scan_physical_plan(
                scan,
                registry,
                runtime,
                extension_codec,
            ),
            PhysicalPlanType::CoalesceBatches(coalesce_batches) => self
                .try_into_coalesce_batches_physical_plan(
                    coalesce_batches,
                    registry,
                    runtime,
                    extension_codec,
                ),
            PhysicalPlanType::Merge(merge) => self.try_into_merge_physical_plan(
                merge,
                registry,
                runtime,
                extension_codec,
            ),
            PhysicalPlanType::Repartition(repart) => self
                .try_into_repartition_physical_plan(
                    repart,
                    registry,
                    runtime,
                    extension_codec,
                ),
            PhysicalPlanType::GlobalLimit(limit) => self
                .try_into_global_limit_physical_plan(
                    limit,
                    registry,
                    runtime,
                    extension_codec,
                ),
            PhysicalPlanType::LocalLimit(limit) => self
                .try_into_local_limit_physical_plan(
                    limit,
                    registry,
                    runtime,
                    extension_codec,
                ),
            PhysicalPlanType::Window(window_agg) => self.try_into_window_physical_plan(
                window_agg,
                registry,
                runtime,
                extension_codec,
            ),
            PhysicalPlanType::Aggregate(hash_agg) => self
                .try_into_aggregate_physical_plan(
                    hash_agg,
                    registry,
                    runtime,
                    extension_codec,
                ),
            PhysicalPlanType::HashJoin(hashjoin) => self
                .try_into_hash_join_physical_plan(
                    hashjoin,
                    registry,
                    runtime,
                    extension_codec,
                ),
            PhysicalPlanType::SymmetricHashJoin(sym_join) => self
                .try_into_symmetric_hash_join_physical_plan(
                    sym_join,
                    registry,
                    runtime,
                    extension_codec,
                ),
            PhysicalPlanType::Union(union) => self.try_into_union_physical_plan(
                union,
                registry,
                runtime,
                extension_codec,
            ),
            PhysicalPlanType::Interleave(interleave) => self
                .try_into_interleave_physical_plan(
                    interleave,
                    registry,
                    runtime,
                    extension_codec,
                ),
            PhysicalPlanType::CrossJoin(crossjoin) => self
                .try_into_cross_join_physical_plan(
                    crossjoin,
                    registry,
                    runtime,
                    extension_codec,
                ),
            PhysicalPlanType::Empty(empty) => self.try_into_empty_physical_plan(
                empty,
                registry,
                runtime,
                extension_codec,
            ),
            PhysicalPlanType::PlaceholderRow(placeholder) => self
                .try_into_placeholder_row_physical_plan(
                    placeholder,
                    registry,
                    runtime,
                    extension_codec,
                ),
            PhysicalPlanType::Sort(sort) => {
                self.try_into_sort_physical_plan(sort, registry, runtime, extension_codec)
            }
            PhysicalPlanType::SortPreservingMerge(sort) => self
                .try_into_sort_preserving_merge_physical_plan(
                    sort,
                    registry,
                    runtime,
                    extension_codec,
                ),
            PhysicalPlanType::Extension(extension) => self
                .try_into_extension_physical_plan(
                    extension,
                    registry,
                    runtime,
                    extension_codec,
                ),
            PhysicalPlanType::NestedLoopJoin(join) => self
                .try_into_nested_loop_join_physical_plan(
                    join,
                    registry,
                    runtime,
                    extension_codec,
                ),
            PhysicalPlanType::Analyze(analyze) => self.try_into_analyze_physical_plan(
                analyze,
                registry,
                runtime,
                extension_codec,
            ),
            PhysicalPlanType::JsonSink(sink) => self.try_into_json_sink_physical_plan(
                sink,
                registry,
                runtime,
                extension_codec,
            ),
            PhysicalPlanType::CsvSink(sink) => self.try_into_csv_sink_physical_plan(
                sink,
                registry,
                runtime,
                extension_codec,
            ),
            #[cfg_attr(not(feature = "parquet"), allow(unused_variables))]
            PhysicalPlanType::ParquetSink(sink) => self
                .try_into_parquet_sink_physical_plan(
                    sink,
                    registry,
                    runtime,
                    extension_codec,
                ),
            PhysicalPlanType::Unnest(unnest) => self.try_into_unnest_physical_plan(
                unnest,
                registry,
                runtime,
                extension_codec,
            ),
            PhysicalPlanType::Cooperative(cooperative) => self
                .try_into_cooperative_physical_plan(
                    cooperative,
                    registry,
                    runtime,
                    extension_codec,
                ),
        }
    }

    fn try_from_physical_plan(
        plan: Arc<dyn ExecutionPlan>,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self>
    where
        Self: Sized,
    {
        let plan_clone = Arc::clone(&plan);
        let plan = plan.as_any();

        if let Some(exec) = plan.downcast_ref::<ExplainExec>() {
            return protobuf::PhysicalPlanNode::try_from_explain_exec(
                exec,
                extension_codec,
            );
        }

        if let Some(exec) = plan.downcast_ref::<ProjectionExec>() {
            return protobuf::PhysicalPlanNode::try_from_projection_exec(
                exec,
                extension_codec,
            );
        }

        if let Some(exec) = plan.downcast_ref::<AnalyzeExec>() {
            return protobuf::PhysicalPlanNode::try_from_analyze_exec(
                exec,
                extension_codec,
            );
        }

        if let Some(exec) = plan.downcast_ref::<FilterExec>() {
            return protobuf::PhysicalPlanNode::try_from_filter_exec(
                exec,
                extension_codec,
            );
        }

        if let Some(limit) = plan.downcast_ref::<GlobalLimitExec>() {
            return protobuf::PhysicalPlanNode::try_from_global_limit_exec(
                limit,
                extension_codec,
            );
        }

        if let Some(limit) = plan.downcast_ref::<LocalLimitExec>() {
            return protobuf::PhysicalPlanNode::try_from_local_limit_exec(
                limit,
                extension_codec,
            );
        }

        if let Some(exec) = plan.downcast_ref::<HashJoinExec>() {
            return protobuf::PhysicalPlanNode::try_from_hash_join_exec(
                exec,
                extension_codec,
            );
        }

        if let Some(exec) = plan.downcast_ref::<SymmetricHashJoinExec>() {
            return protobuf::PhysicalPlanNode::try_from_symmetric_hash_join_exec(
                exec,
                extension_codec,
            );
        }

        if let Some(exec) = plan.downcast_ref::<CrossJoinExec>() {
            return protobuf::PhysicalPlanNode::try_from_cross_join_exec(
                exec,
                extension_codec,
            );
        }

        if let Some(exec) = plan.downcast_ref::<AggregateExec>() {
            return protobuf::PhysicalPlanNode::try_from_aggregate_exec(
                exec,
                extension_codec,
            );
        }

        if let Some(empty) = plan.downcast_ref::<EmptyExec>() {
            return protobuf::PhysicalPlanNode::try_from_empty_exec(
                empty,
                extension_codec,
            );
        }

        if let Some(empty) = plan.downcast_ref::<PlaceholderRowExec>() {
            return protobuf::PhysicalPlanNode::try_from_placeholder_row_exec(
                empty,
                extension_codec,
            );
        }

        if let Some(coalesce_batches) = plan.downcast_ref::<CoalesceBatchesExec>() {
            return protobuf::PhysicalPlanNode::try_from_coalesce_batches_exec(
                coalesce_batches,
                extension_codec,
            );
        }

        if let Some(data_source_exec) = plan.downcast_ref::<DataSourceExec>() {
            if let Some(node) = protobuf::PhysicalPlanNode::try_from_data_source_exec(
                data_source_exec,
                extension_codec,
            )? {
                return Ok(node);
            }
        }

        if let Some(exec) = plan.downcast_ref::<CoalescePartitionsExec>() {
            return protobuf::PhysicalPlanNode::try_from_coalesce_partitions_exec(
                exec,
                extension_codec,
            );
        }

        if let Some(exec) = plan.downcast_ref::<RepartitionExec>() {
            return protobuf::PhysicalPlanNode::try_from_repartition_exec(
                exec,
                extension_codec,
            );
        }

        if let Some(exec) = plan.downcast_ref::<SortExec>() {
            return protobuf::PhysicalPlanNode::try_from_sort_exec(exec, extension_codec);
        }

        if let Some(union) = plan.downcast_ref::<UnionExec>() {
            return protobuf::PhysicalPlanNode::try_from_union_exec(
                union,
                extension_codec,
            );
        }

        if let Some(interleave) = plan.downcast_ref::<InterleaveExec>() {
            return protobuf::PhysicalPlanNode::try_from_interleave_exec(
                interleave,
                extension_codec,
            );
        }

        if let Some(exec) = plan.downcast_ref::<SortPreservingMergeExec>() {
            return protobuf::PhysicalPlanNode::try_from_sort_preserving_merge_exec(
                exec,
                extension_codec,
            );
        }

        if let Some(exec) = plan.downcast_ref::<NestedLoopJoinExec>() {
            return protobuf::PhysicalPlanNode::try_from_nested_loop_join_exec(
                exec,
                extension_codec,
            );
        }

        if let Some(exec) = plan.downcast_ref::<WindowAggExec>() {
            return protobuf::PhysicalPlanNode::try_from_window_agg_exec(
                exec,
                extension_codec,
            );
        }

        if let Some(exec) = plan.downcast_ref::<BoundedWindowAggExec>() {
            return protobuf::PhysicalPlanNode::try_from_bounded_window_agg_exec(
                exec,
                extension_codec,
            );
        }

        if let Some(exec) = plan.downcast_ref::<DataSinkExec>() {
            if let Some(node) = protobuf::PhysicalPlanNode::try_from_data_sink_exec(
                exec,
                extension_codec,
            )? {
                return Ok(node);
            }
        }

        if let Some(exec) = plan.downcast_ref::<UnnestExec>() {
            return protobuf::PhysicalPlanNode::try_from_unnest_exec(
                exec,
                extension_codec,
            );
        }

        if let Some(exec) = plan.downcast_ref::<CooperativeExec>() {
            return protobuf::PhysicalPlanNode::try_from_cooperative_exec(
                exec,
                extension_codec,
            );
        }

        let mut buf: Vec<u8> = vec![];
        match extension_codec.try_encode(Arc::clone(&plan_clone), &mut buf) {
            Ok(_) => {
                let inputs: Vec<protobuf::PhysicalPlanNode> = plan_clone
                    .children()
                    .into_iter()
                    .cloned()
                    .map(|i| {
                        protobuf::PhysicalPlanNode::try_from_physical_plan(
                            i,
                            extension_codec,
                        )
                    })
                    .collect::<Result<_>>()?;

                Ok(protobuf::PhysicalPlanNode {
                    physical_plan_type: Some(PhysicalPlanType::Extension(
                        protobuf::PhysicalExtensionNode { node: buf, inputs },
                    )),
                })
            }
            Err(e) => internal_err!(
                "Unsupported plan and extension codec failed with [{e}]. Plan: {plan_clone:?}"
            ),
        }
    }
}

impl protobuf::PhysicalPlanNode {
    fn try_into_explain_physical_plan(
        &self,
        explain: &protobuf::ExplainExecNode,
        _registry: &dyn FunctionRegistry,
        _runtime: &RuntimeEnv,
        _extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        Ok(Arc::new(ExplainExec::new(
            Arc::new(explain.schema.as_ref().unwrap().try_into()?),
            explain
                .stringified_plans
                .iter()
                .map(|plan| plan.into())
                .collect(),
            explain.verbose,
        )))
    }

    fn try_into_projection_physical_plan(
        &self,
        projection: &protobuf::ProjectionExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let input: Arc<dyn ExecutionPlan> =
            into_physical_plan(&projection.input, registry, runtime, extension_codec)?;
        let exprs = projection
            .expr
            .iter()
            .zip(projection.expr_name.iter())
            .map(|(expr, name)| {
                Ok((
                    parse_physical_expr(
                        expr,
                        registry,
                        input.schema().as_ref(),
                        extension_codec,
                    )?,
                    name.to_string(),
                ))
            })
            .collect::<Result<Vec<(Arc<dyn PhysicalExpr>, String)>>>()?;
        Ok(Arc::new(ProjectionExec::try_new(exprs, input)?))
    }

    fn try_into_filter_physical_plan(
        &self,
        filter: &protobuf::FilterExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let input: Arc<dyn ExecutionPlan> =
            into_physical_plan(&filter.input, registry, runtime, extension_codec)?;
        let projection = if !filter.projection.is_empty() {
            Some(
                filter
                    .projection
                    .iter()
                    .map(|i| *i as usize)
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };

        // Use the projected schema if projection is present, otherwise use the full schema
        let predicate_schema = if let Some(ref proj_indices) = projection {
            // Create projected schema for parsing the predicate
            let projected_fields: Vec<_> = proj_indices
                .iter()
                .map(|&i| input.schema().field(i).clone())
                .collect();
            Arc::new(Schema::new(projected_fields))
        } else {
            input.schema()
        };

        let predicate = filter
            .expr
            .as_ref()
            .map(|expr| {
                parse_physical_expr(
                    expr,
                    registry,
                    predicate_schema.as_ref(),
                    extension_codec,
                )
            })
            .transpose()?
            .ok_or_else(|| {
                DataFusionError::Internal(
                    "filter (FilterExecNode) in PhysicalPlanNode is missing.".to_owned(),
                )
            })?;
        let filter_selectivity = filter.default_filter_selectivity.try_into();
        let filter =
            FilterExec::try_new(predicate, input)?.with_projection(projection)?;
        match filter_selectivity {
            Ok(filter_selectivity) => Ok(Arc::new(
                filter.with_default_selectivity(filter_selectivity)?,
            )),
            Err(_) => Err(DataFusionError::Internal(
                "filter_selectivity in PhysicalPlanNode is invalid ".to_owned(),
            )),
        }
    }

    fn try_into_csv_scan_physical_plan(
        &self,
        scan: &protobuf::CsvScanExecNode,
        registry: &dyn FunctionRegistry,
        _runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let escape =
            if let Some(protobuf::csv_scan_exec_node::OptionalEscape::Escape(escape)) =
                &scan.optional_escape
            {
                Some(str_to_byte(escape, "escape")?)
            } else {
                None
            };

        let comment = if let Some(
            protobuf::csv_scan_exec_node::OptionalComment::Comment(comment),
        ) = &scan.optional_comment
        {
            Some(str_to_byte(comment, "comment")?)
        } else {
            None
        };

        let source = Arc::new(
            CsvSource::new(
                scan.has_header,
                str_to_byte(&scan.delimiter, "delimiter")?,
                0,
            )
            .with_escape(escape)
            .with_comment(comment),
        );

        let conf = FileScanConfigBuilder::from(parse_protobuf_file_scan_config(
            scan.base_conf.as_ref().unwrap(),
            registry,
            extension_codec,
            source,
        )?)
        .with_newlines_in_values(scan.newlines_in_values)
        .with_file_compression_type(FileCompressionType::UNCOMPRESSED)
        .build();
        Ok(DataSourceExec::from_data_source(conf))
    }

    fn try_into_json_scan_physical_plan(
        &self,
        scan: &protobuf::JsonScanExecNode,
        registry: &dyn FunctionRegistry,
        _runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let scan_conf = parse_protobuf_file_scan_config(
            scan.base_conf.as_ref().unwrap(),
            registry,
            extension_codec,
            Arc::new(JsonSource::new()),
        )?;
        Ok(DataSourceExec::from_data_source(scan_conf))
    }

    #[cfg_attr(not(feature = "parquet"), allow(unused_variables))]
    fn try_into_parquet_scan_physical_plan(
        &self,
        scan: &protobuf::ParquetScanExecNode,
        registry: &dyn FunctionRegistry,
        _runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        #[cfg(feature = "parquet")]
        {
            let schema =
                parse_protobuf_file_scan_schema(scan.base_conf.as_ref().unwrap())?;

            // Check if there's a projection and use projected schema for predicate parsing
            let base_conf = scan.base_conf.as_ref().unwrap();
            let predicate_schema = if !base_conf.projection.is_empty() {
                // Create projected schema for parsing the predicate
                let projected_fields: Vec<_> = base_conf
                    .projection
                    .iter()
                    .map(|&i| schema.field(i as usize).clone())
                    .collect();
                Arc::new(Schema::new(projected_fields))
            } else {
                schema
            };

            let predicate = scan
                .predicate
                .as_ref()
                .map(|expr| {
                    parse_physical_expr(
                        expr,
                        registry,
                        predicate_schema.as_ref(),
                        extension_codec,
                    )
                })
                .transpose()?;
            let mut options = TableParquetOptions::default();

            if let Some(table_options) = scan.parquet_options.as_ref() {
                options = table_options.try_into()?;
            }
            let mut source = ParquetSource::new(options);

            if let Some(predicate) = predicate {
                source = source.with_predicate(predicate);
            }
            let base_config = parse_protobuf_file_scan_config(
                base_conf,
                registry,
                extension_codec,
                Arc::new(source),
            )?;
            Ok(DataSourceExec::from_data_source(base_config))
        }
        #[cfg(not(feature = "parquet"))]
        panic!("Unable to process a Parquet PhysicalPlan when `parquet` feature is not enabled")
    }

    #[cfg_attr(not(feature = "avro"), allow(unused_variables))]
    fn try_into_avro_scan_physical_plan(
        &self,
        scan: &protobuf::AvroScanExecNode,
        registry: &dyn FunctionRegistry,
        _runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        #[cfg(feature = "avro")]
        {
            let conf = parse_protobuf_file_scan_config(
                scan.base_conf.as_ref().unwrap(),
                registry,
                extension_codec,
                Arc::new(AvroSource::new()),
            )?;
            Ok(DataSourceExec::from_data_source(conf))
        }
        #[cfg(not(feature = "avro"))]
        panic!("Unable to process a Avro PhysicalPlan when `avro` feature is not enabled")
    }

    fn try_into_coalesce_batches_physical_plan(
        &self,
        coalesce_batches: &protobuf::CoalesceBatchesExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let input: Arc<dyn ExecutionPlan> = into_physical_plan(
            &coalesce_batches.input,
            registry,
            runtime,
            extension_codec,
        )?;
        Ok(Arc::new(
            CoalesceBatchesExec::new(input, coalesce_batches.target_batch_size as usize)
                .with_fetch(coalesce_batches.fetch.map(|f| f as usize)),
        ))
    }

    fn try_into_merge_physical_plan(
        &self,
        merge: &protobuf::CoalescePartitionsExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let input: Arc<dyn ExecutionPlan> =
            into_physical_plan(&merge.input, registry, runtime, extension_codec)?;
        Ok(Arc::new(
            CoalescePartitionsExec::new(input)
                .with_fetch(merge.fetch.map(|f| f as usize)),
        ))
    }

    fn try_into_repartition_physical_plan(
        &self,
        repart: &protobuf::RepartitionExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let input: Arc<dyn ExecutionPlan> =
            into_physical_plan(&repart.input, registry, runtime, extension_codec)?;
        let partitioning = parse_protobuf_partitioning(
            repart.partitioning.as_ref(),
            registry,
            input.schema().as_ref(),
            extension_codec,
        )?;
        Ok(Arc::new(RepartitionExec::try_new(
            input,
            partitioning.unwrap(),
        )?))
    }

    fn try_into_global_limit_physical_plan(
        &self,
        limit: &protobuf::GlobalLimitExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let input: Arc<dyn ExecutionPlan> =
            into_physical_plan(&limit.input, registry, runtime, extension_codec)?;
        let fetch = if limit.fetch >= 0 {
            Some(limit.fetch as usize)
        } else {
            None
        };
        Ok(Arc::new(GlobalLimitExec::new(
            input,
            limit.skip as usize,
            fetch,
        )))
    }

    fn try_into_local_limit_physical_plan(
        &self,
        limit: &protobuf::LocalLimitExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let input: Arc<dyn ExecutionPlan> =
            into_physical_plan(&limit.input, registry, runtime, extension_codec)?;
        Ok(Arc::new(LocalLimitExec::new(input, limit.fetch as usize)))
    }

    fn try_into_window_physical_plan(
        &self,
        window_agg: &protobuf::WindowAggExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let input: Arc<dyn ExecutionPlan> =
            into_physical_plan(&window_agg.input, registry, runtime, extension_codec)?;
        let input_schema = input.schema();

        let physical_window_expr: Vec<Arc<dyn WindowExpr>> = window_agg
            .window_expr
            .iter()
            .map(|window_expr| {
                parse_physical_window_expr(
                    window_expr,
                    registry,
                    input_schema.as_ref(),
                    extension_codec,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        let partition_keys = window_agg
            .partition_keys
            .iter()
            .map(|expr| {
                parse_physical_expr(
                    expr,
                    registry,
                    input.schema().as_ref(),
                    extension_codec,
                )
            })
            .collect::<Result<Vec<Arc<dyn PhysicalExpr>>>>()?;

        if let Some(input_order_mode) = window_agg.input_order_mode.as_ref() {
            let input_order_mode = match input_order_mode {
                window_agg_exec_node::InputOrderMode::Linear(_) => InputOrderMode::Linear,
                window_agg_exec_node::InputOrderMode::PartiallySorted(
                    protobuf::PartiallySortedInputOrderMode { columns },
                ) => InputOrderMode::PartiallySorted(
                    columns.iter().map(|c| *c as usize).collect(),
                ),
                window_agg_exec_node::InputOrderMode::Sorted(_) => InputOrderMode::Sorted,
            };

            Ok(Arc::new(BoundedWindowAggExec::try_new(
                physical_window_expr,
                input,
                input_order_mode,
                !partition_keys.is_empty(),
            )?))
        } else {
            Ok(Arc::new(WindowAggExec::try_new(
                physical_window_expr,
                input,
                !partition_keys.is_empty(),
            )?))
        }
    }

    fn try_into_aggregate_physical_plan(
        &self,
        hash_agg: &protobuf::AggregateExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let input: Arc<dyn ExecutionPlan> =
            into_physical_plan(&hash_agg.input, registry, runtime, extension_codec)?;
        let mode = protobuf::AggregateMode::try_from(hash_agg.mode).map_err(|_| {
            proto_error(format!(
                "Received a AggregateNode message with unknown AggregateMode {}",
                hash_agg.mode
            ))
        })?;
        let agg_mode: AggregateMode = match mode {
            protobuf::AggregateMode::Partial => AggregateMode::Partial,
            protobuf::AggregateMode::Final => AggregateMode::Final,
            protobuf::AggregateMode::FinalPartitioned => AggregateMode::FinalPartitioned,
            protobuf::AggregateMode::Single => AggregateMode::Single,
            protobuf::AggregateMode::SinglePartitioned => {
                AggregateMode::SinglePartitioned
            }
        };

        let num_expr = hash_agg.group_expr.len();

        let group_expr = hash_agg
            .group_expr
            .iter()
            .zip(hash_agg.group_expr_name.iter())
            .map(|(expr, name)| {
                parse_physical_expr(
                    expr,
                    registry,
                    input.schema().as_ref(),
                    extension_codec,
                )
                .map(|expr| (expr, name.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let null_expr = hash_agg
            .null_expr
            .iter()
            .zip(hash_agg.group_expr_name.iter())
            .map(|(expr, name)| {
                parse_physical_expr(
                    expr,
                    registry,
                    input.schema().as_ref(),
                    extension_codec,
                )
                .map(|expr| (expr, name.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let groups: Vec<Vec<bool>> = if !hash_agg.groups.is_empty() {
            hash_agg
                .groups
                .chunks(num_expr)
                .map(|g| g.to_vec())
                .collect::<Vec<Vec<bool>>>()
        } else {
            vec![]
        };

        let input_schema = hash_agg.input_schema.as_ref().ok_or_else(|| {
            DataFusionError::Internal(
                "input_schema in AggregateNode is missing.".to_owned(),
            )
        })?;
        let physical_schema: SchemaRef = SchemaRef::new(input_schema.try_into()?);

        let physical_filter_expr = hash_agg
            .filter_expr
            .iter()
            .map(|expr| {
                expr.expr
                    .as_ref()
                    .map(|e| {
                        parse_physical_expr(
                            e,
                            registry,
                            &physical_schema,
                            extension_codec,
                        )
                    })
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;

        let physical_aggr_expr: Vec<Arc<AggregateFunctionExpr>> = hash_agg
            .aggr_expr
            .iter()
            .zip(hash_agg.aggr_expr_name.iter())
            .map(|(expr, name)| {
                let expr_type = expr.expr_type.as_ref().ok_or_else(|| {
                    proto_error("Unexpected empty aggregate physical expression")
                })?;

                match expr_type {
                    ExprType::AggregateExpr(agg_node) => {
                        let input_phy_expr: Vec<Arc<dyn PhysicalExpr>> = agg_node
                            .expr
                            .iter()
                            .map(|e| {
                                parse_physical_expr(
                                    e,
                                    registry,
                                    &physical_schema,
                                    extension_codec,
                                )
                            })
                            .collect::<Result<Vec<_>>>()?;
                        let order_bys = agg_node
                            .ordering_req
                            .iter()
                            .map(|e| {
                                parse_physical_sort_expr(
                                    e,
                                    registry,
                                    &physical_schema,
                                    extension_codec,
                                )
                            })
                            .collect::<Result<_>>()?;
                        agg_node
                            .aggregate_function
                            .as_ref()
                            .map(|func| match func {
                                AggregateFunction::UserDefinedAggrFunction(udaf_name) => {
                                    let agg_udf = match &agg_node.fun_definition {
                                        Some(buf) => extension_codec
                                            .try_decode_udaf(udaf_name, buf)?,
                                        None => {
                                            registry.udaf(udaf_name).or_else(|_| {
                                                extension_codec
                                                    .try_decode_udaf(udaf_name, &[])
                                            })?
                                        }
                                    };

                                    AggregateExprBuilder::new(agg_udf, input_phy_expr)
                                        .schema(Arc::clone(&physical_schema))
                                        .alias(name)
                                        .human_display(agg_node.human_display.clone())
                                        .with_ignore_nulls(agg_node.ignore_nulls)
                                        .with_distinct(agg_node.distinct)
                                        .order_by(order_bys)
                                        .build()
                                        .map(Arc::new)
                                }
                            })
                            .transpose()?
                            .ok_or_else(|| {
                                proto_error(
                                    "Invalid AggregateExpr, missing aggregate_function",
                                )
                            })
                    }
                    _ => internal_err!("Invalid aggregate expression for AggregateExec"),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        let limit = hash_agg
            .limit
            .as_ref()
            .map(|lit_value| lit_value.limit as usize);

        let agg = AggregateExec::try_new(
            agg_mode,
            PhysicalGroupBy::new(group_expr, null_expr, groups),
            physical_aggr_expr,
            physical_filter_expr,
            input,
            physical_schema,
        )?;

        let agg = agg.with_limit(limit);

        Ok(Arc::new(agg))
    }

    fn try_into_hash_join_physical_plan(
        &self,
        hashjoin: &protobuf::HashJoinExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let left: Arc<dyn ExecutionPlan> =
            into_physical_plan(&hashjoin.left, registry, runtime, extension_codec)?;
        let right: Arc<dyn ExecutionPlan> =
            into_physical_plan(&hashjoin.right, registry, runtime, extension_codec)?;
        let left_schema = left.schema();
        let right_schema = right.schema();
        let on: Vec<(PhysicalExprRef, PhysicalExprRef)> = hashjoin
            .on
            .iter()
            .map(|col| {
                let left = parse_physical_expr(
                    &col.left.clone().unwrap(),
                    registry,
                    left_schema.as_ref(),
                    extension_codec,
                )?;
                let right = parse_physical_expr(
                    &col.right.clone().unwrap(),
                    registry,
                    right_schema.as_ref(),
                    extension_codec,
                )?;
                Ok((left, right))
            })
            .collect::<Result<_>>()?;
        let join_type =
            protobuf::JoinType::try_from(hashjoin.join_type).map_err(|_| {
                proto_error(format!(
                    "Received a HashJoinNode message with unknown JoinType {}",
                    hashjoin.join_type
                ))
            })?;
        let null_equality = protobuf::NullEquality::try_from(hashjoin.null_equality)
            .map_err(|_| {
                proto_error(format!(
                    "Received a HashJoinNode message with unknown NullEquality {}",
                    hashjoin.null_equality
                ))
            })?;
        let filter = hashjoin
            .filter
            .as_ref()
            .map(|f| {
                let schema = f
                    .schema
                    .as_ref()
                    .ok_or_else(|| proto_error("Missing JoinFilter schema"))?
                    .try_into()?;

                let expression = parse_physical_expr(
                    f.expression.as_ref().ok_or_else(|| {
                        proto_error("Unexpected empty filter expression")
                    })?,
                    registry, &schema,
                    extension_codec,
                )?;
                let column_indices = f.column_indices
                    .iter()
                    .map(|i| {
                        let side = protobuf::JoinSide::try_from(i.side)
                            .map_err(|_| proto_error(format!(
                                "Received a HashJoinNode message with JoinSide in Filter {}",
                                i.side))
                            )?;

                        Ok(ColumnIndex {
                            index: i.index as usize,
                            side: side.into(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;

                Ok(JoinFilter::new(expression, column_indices, Arc::new(schema)))
            })
            .map_or(Ok(None), |v: Result<JoinFilter>| v.map(Some))?;

        let partition_mode = protobuf::PartitionMode::try_from(hashjoin.partition_mode)
            .map_err(|_| {
            proto_error(format!(
                "Received a HashJoinNode message with unknown PartitionMode {}",
                hashjoin.partition_mode
            ))
        })?;
        let partition_mode = match partition_mode {
            protobuf::PartitionMode::CollectLeft => PartitionMode::CollectLeft,
            protobuf::PartitionMode::Partitioned => PartitionMode::Partitioned,
            protobuf::PartitionMode::Auto => PartitionMode::Auto,
        };
        let projection = if !hashjoin.projection.is_empty() {
            Some(
                hashjoin
                    .projection
                    .iter()
                    .map(|i| *i as usize)
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };
        Ok(Arc::new(HashJoinExec::try_new(
            left,
            right,
            on,
            filter,
            &join_type.into(),
            projection,
            partition_mode,
            null_equality.into(),
        )?))
    }

    fn try_into_symmetric_hash_join_physical_plan(
        &self,
        sym_join: &protobuf::SymmetricHashJoinExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let left =
            into_physical_plan(&sym_join.left, registry, runtime, extension_codec)?;
        let right =
            into_physical_plan(&sym_join.right, registry, runtime, extension_codec)?;
        let left_schema = left.schema();
        let right_schema = right.schema();
        let on = sym_join
            .on
            .iter()
            .map(|col| {
                let left = parse_physical_expr(
                    &col.left.clone().unwrap(),
                    registry,
                    left_schema.as_ref(),
                    extension_codec,
                )?;
                let right = parse_physical_expr(
                    &col.right.clone().unwrap(),
                    registry,
                    right_schema.as_ref(),
                    extension_codec,
                )?;
                Ok((left, right))
            })
            .collect::<Result<_>>()?;
        let join_type =
            protobuf::JoinType::try_from(sym_join.join_type).map_err(|_| {
                proto_error(format!(
                    "Received a SymmetricHashJoin message with unknown JoinType {}",
                    sym_join.join_type
                ))
            })?;
        let null_equality = protobuf::NullEquality::try_from(sym_join.null_equality)
            .map_err(|_| {
                proto_error(format!(
                    "Received a SymmetricHashJoin message with unknown NullEquality {}",
                    sym_join.null_equality
                ))
            })?;
        let filter = sym_join
            .filter
            .as_ref()
            .map(|f| {
                let schema = f
                    .schema
                    .as_ref()
                    .ok_or_else(|| proto_error("Missing JoinFilter schema"))?
                    .try_into()?;

                let expression = parse_physical_expr(
                    f.expression.as_ref().ok_or_else(|| {
                        proto_error("Unexpected empty filter expression")
                    })?,
                    registry, &schema,
                    extension_codec,
                )?;
                let column_indices = f.column_indices
                    .iter()
                    .map(|i| {
                        let side = protobuf::JoinSide::try_from(i.side)
                            .map_err(|_| proto_error(format!(
                                "Received a HashJoinNode message with JoinSide in Filter {}",
                                i.side))
                            )?;

                        Ok(ColumnIndex {
                            index: i.index as usize,
                            side: side.into(),
                        })
                    })
                    .collect::<Result<_>>()?;

                Ok(JoinFilter::new(expression, column_indices, Arc::new(schema)))
            })
            .map_or(Ok(None), |v: Result<JoinFilter>| v.map(Some))?;

        let left_sort_exprs = parse_physical_sort_exprs(
            &sym_join.left_sort_exprs,
            registry,
            &left_schema,
            extension_codec,
        )?;
        let left_sort_exprs = LexOrdering::new(left_sort_exprs);

        let right_sort_exprs = parse_physical_sort_exprs(
            &sym_join.right_sort_exprs,
            registry,
            &right_schema,
            extension_codec,
        )?;
        let right_sort_exprs = LexOrdering::new(right_sort_exprs);

        let partition_mode = protobuf::StreamPartitionMode::try_from(
            sym_join.partition_mode,
        )
        .map_err(|_| {
            proto_error(format!(
                "Received a SymmetricHashJoin message with unknown PartitionMode {}",
                sym_join.partition_mode
            ))
        })?;
        let partition_mode = match partition_mode {
            protobuf::StreamPartitionMode::SinglePartition => {
                StreamJoinPartitionMode::SinglePartition
            }
            protobuf::StreamPartitionMode::PartitionedExec => {
                StreamJoinPartitionMode::Partitioned
            }
        };
        SymmetricHashJoinExec::try_new(
            left,
            right,
            on,
            filter,
            &join_type.into(),
            null_equality.into(),
            left_sort_exprs,
            right_sort_exprs,
            partition_mode,
        )
        .map(|e| Arc::new(e) as _)
    }

    fn try_into_union_physical_plan(
        &self,
        union: &protobuf::UnionExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let mut inputs: Vec<Arc<dyn ExecutionPlan>> = vec![];
        for input in &union.inputs {
            inputs.push(input.try_into_physical_plan(
                registry,
                runtime,
                extension_codec,
            )?);
        }
        Ok(Arc::new(UnionExec::new(inputs)))
    }

    fn try_into_interleave_physical_plan(
        &self,
        interleave: &protobuf::InterleaveExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let mut inputs: Vec<Arc<dyn ExecutionPlan>> = vec![];
        for input in &interleave.inputs {
            inputs.push(input.try_into_physical_plan(
                registry,
                runtime,
                extension_codec,
            )?);
        }
        Ok(Arc::new(InterleaveExec::try_new(inputs)?))
    }

    fn try_into_cross_join_physical_plan(
        &self,
        crossjoin: &protobuf::CrossJoinExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let left: Arc<dyn ExecutionPlan> =
            into_physical_plan(&crossjoin.left, registry, runtime, extension_codec)?;
        let right: Arc<dyn ExecutionPlan> =
            into_physical_plan(&crossjoin.right, registry, runtime, extension_codec)?;
        Ok(Arc::new(CrossJoinExec::new(left, right)))
    }

    fn try_into_empty_physical_plan(
        &self,
        empty: &protobuf::EmptyExecNode,
        _registry: &dyn FunctionRegistry,
        _runtime: &RuntimeEnv,
        _extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let schema = Arc::new(convert_required!(empty.schema)?);
        Ok(Arc::new(EmptyExec::new(schema)))
    }

    fn try_into_placeholder_row_physical_plan(
        &self,
        placeholder: &protobuf::PlaceholderRowExecNode,
        _registry: &dyn FunctionRegistry,
        _runtime: &RuntimeEnv,
        _extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let schema = Arc::new(convert_required!(placeholder.schema)?);
        Ok(Arc::new(PlaceholderRowExec::new(schema)))
    }

    fn try_into_sort_physical_plan(
        &self,
        sort: &protobuf::SortExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let input = into_physical_plan(&sort.input, registry, runtime, extension_codec)?;
        let exprs = sort
            .expr
            .iter()
            .map(|expr| {
                let expr = expr.expr_type.as_ref().ok_or_else(|| {
                    proto_error(format!(
                        "physical_plan::from_proto() Unexpected expr {self:?}"
                    ))
                })?;
                if let ExprType::Sort(sort_expr) = expr {
                    let expr = sort_expr
                        .expr
                        .as_ref()
                        .ok_or_else(|| {
                            proto_error(format!(
                                "physical_plan::from_proto() Unexpected sort expr {self:?}"
                            ))
                        })?
                        .as_ref();
                    Ok(PhysicalSortExpr {
                        expr: parse_physical_expr(expr, registry, input.schema().as_ref(), extension_codec)?,
                        options: SortOptions {
                            descending: !sort_expr.asc,
                            nulls_first: sort_expr.nulls_first,
                        },
                    })
                } else {
                    internal_err!(
                        "physical_plan::from_proto() {self:?}"
                    )
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let Some(ordering) = LexOrdering::new(exprs) else {
            return internal_err!("SortExec requires an ordering");
        };
        let fetch = (sort.fetch >= 0).then_some(sort.fetch as _);
        let new_sort = SortExec::new(ordering, input)
            .with_fetch(fetch)
            .with_preserve_partitioning(sort.preserve_partitioning);

        Ok(Arc::new(new_sort))
    }

    fn try_into_sort_preserving_merge_physical_plan(
        &self,
        sort: &protobuf::SortPreservingMergeExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let input = into_physical_plan(&sort.input, registry, runtime, extension_codec)?;
        let exprs = sort
            .expr
            .iter()
            .map(|expr| {
                let expr = expr.expr_type.as_ref().ok_or_else(|| {
                    proto_error(format!(
                        "physical_plan::from_proto() Unexpected expr {self:?}"
                    ))
                })?;
                if let ExprType::Sort(sort_expr) = expr {
                    let expr = sort_expr
                        .expr
                        .as_ref()
                        .ok_or_else(|| {
                            proto_error(format!(
                            "physical_plan::from_proto() Unexpected sort expr {self:?}"
                        ))
                        })?
                        .as_ref();
                    Ok(PhysicalSortExpr {
                        expr: parse_physical_expr(
                            expr,
                            registry,
                            input.schema().as_ref(),
                            extension_codec,
                        )?,
                        options: SortOptions {
                            descending: !sort_expr.asc,
                            nulls_first: sort_expr.nulls_first,
                        },
                    })
                } else {
                    internal_err!("physical_plan::from_proto() {self:?}")
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let Some(ordering) = LexOrdering::new(exprs) else {
            return internal_err!("SortExec requires an ordering");
        };
        let fetch = (sort.fetch >= 0).then_some(sort.fetch as _);
        Ok(Arc::new(
            SortPreservingMergeExec::new(ordering, input).with_fetch(fetch),
        ))
    }

    fn try_into_extension_physical_plan(
        &self,
        extension: &protobuf::PhysicalExtensionNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let inputs: Vec<Arc<dyn ExecutionPlan>> = extension
            .inputs
            .iter()
            .map(|i| i.try_into_physical_plan(registry, runtime, extension_codec))
            .collect::<Result<_>>()?;

        let extension_node =
            extension_codec.try_decode(extension.node.as_slice(), &inputs, registry)?;

        Ok(extension_node)
    }

    fn try_into_nested_loop_join_physical_plan(
        &self,
        join: &protobuf::NestedLoopJoinExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let left: Arc<dyn ExecutionPlan> =
            into_physical_plan(&join.left, registry, runtime, extension_codec)?;
        let right: Arc<dyn ExecutionPlan> =
            into_physical_plan(&join.right, registry, runtime, extension_codec)?;
        let join_type = protobuf::JoinType::try_from(join.join_type).map_err(|_| {
            proto_error(format!(
                "Received a NestedLoopJoinExecNode message with unknown JoinType {}",
                join.join_type
            ))
        })?;
        let filter = join
                    .filter
                    .as_ref()
                    .map(|f| {
                        let schema = f
                            .schema
                            .as_ref()
                            .ok_or_else(|| proto_error("Missing JoinFilter schema"))?
                            .try_into()?;

                        let expression = parse_physical_expr(
                            f.expression.as_ref().ok_or_else(|| {
                                proto_error("Unexpected empty filter expression")
                            })?,
                            registry, &schema,
                            extension_codec,
                        )?;
                        let column_indices = f.column_indices
                            .iter()
                            .map(|i| {
                                let side = protobuf::JoinSide::try_from(i.side)
                                    .map_err(|_| proto_error(format!(
                                        "Received a NestedLoopJoinExecNode message with JoinSide in Filter {}",
                                        i.side))
                                    )?;

                                Ok(ColumnIndex {
                                    index: i.index as usize,
                                    side: side.into(),
                                })
                            })
                            .collect::<Result<Vec<_>>>()?;

                        Ok(JoinFilter::new(expression, column_indices, Arc::new(schema)))
                    })
                    .map_or(Ok(None), |v: Result<JoinFilter>| v.map(Some))?;

        let projection = if !join.projection.is_empty() {
            Some(
                join.projection
                    .iter()
                    .map(|i| *i as usize)
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };

        Ok(Arc::new(NestedLoopJoinExec::try_new(
            left,
            right,
            filter,
            &join_type.into(),
            projection,
        )?))
    }

    fn try_into_analyze_physical_plan(
        &self,
        analyze: &protobuf::AnalyzeExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let input: Arc<dyn ExecutionPlan> =
            into_physical_plan(&analyze.input, registry, runtime, extension_codec)?;
        Ok(Arc::new(AnalyzeExec::new(
            analyze.verbose,
            analyze.show_statistics,
            input,
            Arc::new(convert_required!(analyze.schema)?),
        )))
    }

    fn try_into_json_sink_physical_plan(
        &self,
        sink: &protobuf::JsonSinkExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let input = into_physical_plan(&sink.input, registry, runtime, extension_codec)?;

        let data_sink: JsonSink = sink
            .sink
            .as_ref()
            .ok_or_else(|| proto_error("Missing required field in protobuf"))?
            .try_into()?;
        let sink_schema = input.schema();
        let sort_order = sink
            .sort_order
            .as_ref()
            .map(|collection| {
                parse_physical_sort_exprs(
                    &collection.physical_sort_expr_nodes,
                    registry,
                    &sink_schema,
                    extension_codec,
                )
                .map(|sort_exprs| {
                    LexRequirement::new(sort_exprs.into_iter().map(Into::into))
                })
            })
            .transpose()?
            .flatten();
        Ok(Arc::new(DataSinkExec::new(
            input,
            Arc::new(data_sink),
            sort_order,
        )))
    }

    fn try_into_csv_sink_physical_plan(
        &self,
        sink: &protobuf::CsvSinkExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let input = into_physical_plan(&sink.input, registry, runtime, extension_codec)?;

        let data_sink: CsvSink = sink
            .sink
            .as_ref()
            .ok_or_else(|| proto_error("Missing required field in protobuf"))?
            .try_into()?;
        let sink_schema = input.schema();
        let sort_order = sink
            .sort_order
            .as_ref()
            .map(|collection| {
                parse_physical_sort_exprs(
                    &collection.physical_sort_expr_nodes,
                    registry,
                    &sink_schema,
                    extension_codec,
                )
                .map(|sort_exprs| {
                    LexRequirement::new(sort_exprs.into_iter().map(Into::into))
                })
            })
            .transpose()?
            .flatten();
        Ok(Arc::new(DataSinkExec::new(
            input,
            Arc::new(data_sink),
            sort_order,
        )))
    }

    fn try_into_parquet_sink_physical_plan(
        &self,
        sink: &protobuf::ParquetSinkExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        #[cfg(feature = "parquet")]
        {
            let input =
                into_physical_plan(&sink.input, registry, runtime, extension_codec)?;

            let data_sink: ParquetSink = sink
                .sink
                .as_ref()
                .ok_or_else(|| proto_error("Missing required field in protobuf"))?
                .try_into()?;
            let sink_schema = input.schema();
            let sort_order = sink
                .sort_order
                .as_ref()
                .map(|collection| {
                    parse_physical_sort_exprs(
                        &collection.physical_sort_expr_nodes,
                        registry,
                        &sink_schema,
                        extension_codec,
                    )
                    .map(|sort_exprs| {
                        LexRequirement::new(sort_exprs.into_iter().map(Into::into))
                    })
                })
                .transpose()?
                .flatten();
            Ok(Arc::new(DataSinkExec::new(
                input,
                Arc::new(data_sink),
                sort_order,
            )))
        }
        #[cfg(not(feature = "parquet"))]
        panic!("Trying to use ParquetSink without `parquet` feature enabled");
    }

    fn try_into_unnest_physical_plan(
        &self,
        unnest: &protobuf::UnnestExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let input =
            into_physical_plan(&unnest.input, registry, runtime, extension_codec)?;

        Ok(Arc::new(UnnestExec::new(
            input,
            unnest
                .list_type_columns
                .iter()
                .map(|c| ListUnnest {
                    index_in_input_schema: c.index_in_input_schema as _,
                    depth: c.depth as _,
                })
                .collect(),
            unnest.struct_type_columns.iter().map(|c| *c as _).collect(),
            Arc::new(convert_required!(unnest.schema)?),
            into_required!(unnest.options)?,
        )))
    }

    fn try_into_cooperative_physical_plan(
        &self,
        field_stream: &protobuf::CooperativeExecNode,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        let input =
            into_physical_plan(&field_stream.input, registry, runtime, extension_codec)?;
        Ok(Arc::new(CooperativeExec::new(input)))
    }

    fn try_from_explain_exec(
        exec: &ExplainExec,
        _extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::Explain(
                protobuf::ExplainExecNode {
                    schema: Some(exec.schema().as_ref().try_into()?),
                    stringified_plans: exec
                        .stringified_plans()
                        .iter()
                        .map(|plan| plan.into())
                        .collect(),
                    verbose: exec.verbose(),
                },
            )),
        })
    }

    fn try_from_projection_exec(
        exec: &ProjectionExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let input = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.input().to_owned(),
            extension_codec,
        )?;
        let expr = exec
            .expr()
            .iter()
            .map(|expr| serialize_physical_expr(&expr.0, extension_codec))
            .collect::<Result<Vec<_>>>()?;
        let expr_name = exec.expr().iter().map(|expr| expr.1.clone()).collect();
        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::Projection(Box::new(
                protobuf::ProjectionExecNode {
                    input: Some(Box::new(input)),
                    expr,
                    expr_name,
                },
            ))),
        })
    }

    fn try_from_analyze_exec(
        exec: &AnalyzeExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let input = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.input().to_owned(),
            extension_codec,
        )?;
        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::Analyze(Box::new(
                protobuf::AnalyzeExecNode {
                    verbose: exec.verbose(),
                    show_statistics: exec.show_statistics(),
                    input: Some(Box::new(input)),
                    schema: Some(exec.schema().as_ref().try_into()?),
                },
            ))),
        })
    }

    fn try_from_filter_exec(
        exec: &FilterExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let input = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.input().to_owned(),
            extension_codec,
        )?;
        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::Filter(Box::new(
                protobuf::FilterExecNode {
                    input: Some(Box::new(input)),
                    expr: Some(serialize_physical_expr(
                        exec.predicate(),
                        extension_codec,
                    )?),
                    default_filter_selectivity: exec.default_selectivity() as u32,
                    projection: exec.projection().as_ref().map_or_else(Vec::new, |v| {
                        v.iter().map(|x| *x as u32).collect::<Vec<u32>>()
                    }),
                },
            ))),
        })
    }

    fn try_from_global_limit_exec(
        limit: &GlobalLimitExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let input = protobuf::PhysicalPlanNode::try_from_physical_plan(
            limit.input().to_owned(),
            extension_codec,
        )?;

        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::GlobalLimit(Box::new(
                protobuf::GlobalLimitExecNode {
                    input: Some(Box::new(input)),
                    skip: limit.skip() as u32,
                    fetch: match limit.fetch() {
                        Some(n) => n as i64,
                        _ => -1, // no limit
                    },
                },
            ))),
        })
    }

    fn try_from_local_limit_exec(
        limit: &LocalLimitExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let input = protobuf::PhysicalPlanNode::try_from_physical_plan(
            limit.input().to_owned(),
            extension_codec,
        )?;
        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::LocalLimit(Box::new(
                protobuf::LocalLimitExecNode {
                    input: Some(Box::new(input)),
                    fetch: limit.fetch() as u32,
                },
            ))),
        })
    }

    fn try_from_hash_join_exec(
        exec: &HashJoinExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let left = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.left().to_owned(),
            extension_codec,
        )?;
        let right = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.right().to_owned(),
            extension_codec,
        )?;
        let on: Vec<protobuf::JoinOn> = exec
            .on()
            .iter()
            .map(|tuple| {
                let l = serialize_physical_expr(&tuple.0, extension_codec)?;
                let r = serialize_physical_expr(&tuple.1, extension_codec)?;
                Ok::<_, DataFusionError>(protobuf::JoinOn {
                    left: Some(l),
                    right: Some(r),
                })
            })
            .collect::<Result<_>>()?;
        let join_type: protobuf::JoinType = exec.join_type().to_owned().into();
        let null_equality: protobuf::NullEquality = exec.null_equality().into();
        let filter = exec
            .filter()
            .as_ref()
            .map(|f| {
                let expression =
                    serialize_physical_expr(f.expression(), extension_codec)?;
                let column_indices = f
                    .column_indices()
                    .iter()
                    .map(|i| {
                        let side: protobuf::JoinSide = i.side.to_owned().into();
                        protobuf::ColumnIndex {
                            index: i.index as u32,
                            side: side.into(),
                        }
                    })
                    .collect();
                let schema = f.schema().as_ref().try_into()?;
                Ok(protobuf::JoinFilter {
                    expression: Some(expression),
                    column_indices,
                    schema: Some(schema),
                })
            })
            .map_or(Ok(None), |v: Result<protobuf::JoinFilter>| v.map(Some))?;

        let partition_mode = match exec.partition_mode() {
            PartitionMode::CollectLeft => protobuf::PartitionMode::CollectLeft,
            PartitionMode::Partitioned => protobuf::PartitionMode::Partitioned,
            PartitionMode::Auto => protobuf::PartitionMode::Auto,
        };

        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::HashJoin(Box::new(
                protobuf::HashJoinExecNode {
                    left: Some(Box::new(left)),
                    right: Some(Box::new(right)),
                    on,
                    join_type: join_type.into(),
                    partition_mode: partition_mode.into(),
                    null_equality: null_equality.into(),
                    filter,
                    projection: exec.projection.as_ref().map_or_else(Vec::new, |v| {
                        v.iter().map(|x| *x as u32).collect::<Vec<u32>>()
                    }),
                },
            ))),
        })
    }

    fn try_from_symmetric_hash_join_exec(
        exec: &SymmetricHashJoinExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let left = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.left().to_owned(),
            extension_codec,
        )?;
        let right = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.right().to_owned(),
            extension_codec,
        )?;
        let on = exec
            .on()
            .iter()
            .map(|tuple| {
                let l = serialize_physical_expr(&tuple.0, extension_codec)?;
                let r = serialize_physical_expr(&tuple.1, extension_codec)?;
                Ok::<_, DataFusionError>(protobuf::JoinOn {
                    left: Some(l),
                    right: Some(r),
                })
            })
            .collect::<Result<_>>()?;
        let join_type: protobuf::JoinType = exec.join_type().to_owned().into();
        let null_equality: protobuf::NullEquality = exec.null_equality().into();
        let filter = exec
            .filter()
            .as_ref()
            .map(|f| {
                let expression =
                    serialize_physical_expr(f.expression(), extension_codec)?;
                let column_indices = f
                    .column_indices()
                    .iter()
                    .map(|i| {
                        let side: protobuf::JoinSide = i.side.to_owned().into();
                        protobuf::ColumnIndex {
                            index: i.index as u32,
                            side: side.into(),
                        }
                    })
                    .collect();
                let schema = f.schema().as_ref().try_into()?;
                Ok(protobuf::JoinFilter {
                    expression: Some(expression),
                    column_indices,
                    schema: Some(schema),
                })
            })
            .map_or(Ok(None), |v: Result<protobuf::JoinFilter>| v.map(Some))?;

        let partition_mode = match exec.partition_mode() {
            StreamJoinPartitionMode::SinglePartition => {
                protobuf::StreamPartitionMode::SinglePartition
            }
            StreamJoinPartitionMode::Partitioned => {
                protobuf::StreamPartitionMode::PartitionedExec
            }
        };

        let left_sort_exprs = exec
            .left_sort_exprs()
            .map(|exprs| {
                exprs
                    .iter()
                    .map(|expr| {
                        Ok(protobuf::PhysicalSortExprNode {
                            expr: Some(Box::new(serialize_physical_expr(
                                &expr.expr,
                                extension_codec,
                            )?)),
                            asc: !expr.options.descending,
                            nulls_first: expr.options.nulls_first,
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or(vec![]);

        let right_sort_exprs = exec
            .right_sort_exprs()
            .map(|exprs| {
                exprs
                    .iter()
                    .map(|expr| {
                        Ok(protobuf::PhysicalSortExprNode {
                            expr: Some(Box::new(serialize_physical_expr(
                                &expr.expr,
                                extension_codec,
                            )?)),
                            asc: !expr.options.descending,
                            nulls_first: expr.options.nulls_first,
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
            .unwrap_or(vec![]);

        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::SymmetricHashJoin(Box::new(
                protobuf::SymmetricHashJoinExecNode {
                    left: Some(Box::new(left)),
                    right: Some(Box::new(right)),
                    on,
                    join_type: join_type.into(),
                    partition_mode: partition_mode.into(),
                    null_equality: null_equality.into(),
                    left_sort_exprs,
                    right_sort_exprs,
                    filter,
                },
            ))),
        })
    }

    fn try_from_cross_join_exec(
        exec: &CrossJoinExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let left = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.left().to_owned(),
            extension_codec,
        )?;
        let right = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.right().to_owned(),
            extension_codec,
        )?;
        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::CrossJoin(Box::new(
                protobuf::CrossJoinExecNode {
                    left: Some(Box::new(left)),
                    right: Some(Box::new(right)),
                },
            ))),
        })
    }

    fn try_from_aggregate_exec(
        exec: &AggregateExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let groups: Vec<bool> = exec
            .group_expr()
            .groups()
            .iter()
            .flatten()
            .copied()
            .collect();

        let group_names = exec
            .group_expr()
            .expr()
            .iter()
            .map(|expr| expr.1.to_owned())
            .collect();

        let filter = exec
            .filter_expr()
            .iter()
            .map(|expr| serialize_maybe_filter(expr.to_owned(), extension_codec))
            .collect::<Result<Vec<_>>>()?;

        let agg = exec
            .aggr_expr()
            .iter()
            .map(|expr| serialize_physical_aggr_expr(expr.to_owned(), extension_codec))
            .collect::<Result<Vec<_>>>()?;

        let agg_names = exec
            .aggr_expr()
            .iter()
            .map(|expr| expr.name().to_string())
            .collect::<Vec<_>>();

        let agg_mode = match exec.mode() {
            AggregateMode::Partial => protobuf::AggregateMode::Partial,
            AggregateMode::Final => protobuf::AggregateMode::Final,
            AggregateMode::FinalPartitioned => protobuf::AggregateMode::FinalPartitioned,
            AggregateMode::Single => protobuf::AggregateMode::Single,
            AggregateMode::SinglePartitioned => {
                protobuf::AggregateMode::SinglePartitioned
            }
        };
        let input_schema = exec.input_schema();
        let input = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.input().to_owned(),
            extension_codec,
        )?;

        let null_expr = exec
            .group_expr()
            .null_expr()
            .iter()
            .map(|expr| serialize_physical_expr(&expr.0, extension_codec))
            .collect::<Result<Vec<_>>>()?;

        let group_expr = exec
            .group_expr()
            .expr()
            .iter()
            .map(|expr| serialize_physical_expr(&expr.0, extension_codec))
            .collect::<Result<Vec<_>>>()?;

        let limit = exec.limit().map(|value| protobuf::AggLimit {
            limit: value as u64,
        });

        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::Aggregate(Box::new(
                protobuf::AggregateExecNode {
                    group_expr,
                    group_expr_name: group_names,
                    aggr_expr: agg,
                    filter_expr: filter,
                    aggr_expr_name: agg_names,
                    mode: agg_mode as i32,
                    input: Some(Box::new(input)),
                    input_schema: Some(input_schema.as_ref().try_into()?),
                    null_expr,
                    groups,
                    limit,
                },
            ))),
        })
    }

    fn try_from_empty_exec(
        empty: &EmptyExec,
        _extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let schema = empty.schema().as_ref().try_into()?;
        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::Empty(protobuf::EmptyExecNode {
                schema: Some(schema),
            })),
        })
    }

    fn try_from_placeholder_row_exec(
        empty: &PlaceholderRowExec,
        _extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let schema = empty.schema().as_ref().try_into()?;
        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::PlaceholderRow(
                protobuf::PlaceholderRowExecNode {
                    schema: Some(schema),
                },
            )),
        })
    }

    fn try_from_coalesce_batches_exec(
        coalesce_batches: &CoalesceBatchesExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let input = protobuf::PhysicalPlanNode::try_from_physical_plan(
            coalesce_batches.input().to_owned(),
            extension_codec,
        )?;
        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::CoalesceBatches(Box::new(
                protobuf::CoalesceBatchesExecNode {
                    input: Some(Box::new(input)),
                    target_batch_size: coalesce_batches.target_batch_size() as u32,
                    fetch: coalesce_batches.fetch().map(|n| n as u32),
                },
            ))),
        })
    }

    fn try_from_data_source_exec(
        data_source_exec: &DataSourceExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Option<Self>> {
        let data_source = data_source_exec.data_source();
        if let Some(maybe_csv) = data_source.as_any().downcast_ref::<FileScanConfig>() {
            let source = maybe_csv.file_source();
            if let Some(csv_config) = source.as_any().downcast_ref::<CsvSource>() {
                return Ok(Some(protobuf::PhysicalPlanNode {
                    physical_plan_type: Some(PhysicalPlanType::CsvScan(
                        protobuf::CsvScanExecNode {
                            base_conf: Some(serialize_file_scan_config(
                                maybe_csv,
                                extension_codec,
                            )?),
                            has_header: csv_config.has_header(),
                            delimiter: byte_to_string(
                                csv_config.delimiter(),
                                "delimiter",
                            )?,
                            quote: byte_to_string(csv_config.quote(), "quote")?,
                            optional_escape: if let Some(escape) = csv_config.escape() {
                                Some(
                                    protobuf::csv_scan_exec_node::OptionalEscape::Escape(
                                        byte_to_string(escape, "escape")?,
                                    ),
                                )
                            } else {
                                None
                            },
                            optional_comment: if let Some(comment) = csv_config.comment()
                            {
                                Some(protobuf::csv_scan_exec_node::OptionalComment::Comment(
                                        byte_to_string(comment, "comment")?,
                                    ))
                            } else {
                                None
                            },
                            newlines_in_values: maybe_csv.newlines_in_values(),
                        },
                    )),
                }));
            }
        }

        if let Some(scan_conf) = data_source.as_any().downcast_ref::<FileScanConfig>() {
            let source = scan_conf.file_source();
            if let Some(_json_source) = source.as_any().downcast_ref::<JsonSource>() {
                return Ok(Some(protobuf::PhysicalPlanNode {
                    physical_plan_type: Some(PhysicalPlanType::JsonScan(
                        protobuf::JsonScanExecNode {
                            base_conf: Some(serialize_file_scan_config(
                                scan_conf,
                                extension_codec,
                            )?),
                        },
                    )),
                }));
            }
        }

        #[cfg(feature = "parquet")]
        if let Some((maybe_parquet, conf)) =
            data_source_exec.downcast_to_file_source::<ParquetSource>()
        {
            let predicate = conf
                .predicate()
                .map(|pred| serialize_physical_expr(pred, extension_codec))
                .transpose()?;
            return Ok(Some(protobuf::PhysicalPlanNode {
                physical_plan_type: Some(PhysicalPlanType::ParquetScan(
                    protobuf::ParquetScanExecNode {
                        base_conf: Some(serialize_file_scan_config(
                            maybe_parquet,
                            extension_codec,
                        )?),
                        predicate,
                        parquet_options: Some(conf.table_parquet_options().try_into()?),
                    },
                )),
            }));
        }

        #[cfg(feature = "avro")]
        if let Some(maybe_avro) = data_source.as_any().downcast_ref::<FileScanConfig>() {
            let source = maybe_avro.file_source();
            if source.as_any().downcast_ref::<AvroSource>().is_some() {
                return Ok(Some(protobuf::PhysicalPlanNode {
                    physical_plan_type: Some(PhysicalPlanType::AvroScan(
                        protobuf::AvroScanExecNode {
                            base_conf: Some(serialize_file_scan_config(
                                maybe_avro,
                                extension_codec,
                            )?),
                        },
                    )),
                }));
            }
        }

        Ok(None)
    }

    fn try_from_coalesce_partitions_exec(
        exec: &CoalescePartitionsExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let input = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.input().to_owned(),
            extension_codec,
        )?;
        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::Merge(Box::new(
                protobuf::CoalescePartitionsExecNode {
                    input: Some(Box::new(input)),
                    fetch: exec.fetch().map(|f| f as u32),
                },
            ))),
        })
    }

    fn try_from_repartition_exec(
        exec: &RepartitionExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let input = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.input().to_owned(),
            extension_codec,
        )?;

        let pb_partitioning =
            serialize_partitioning(exec.partitioning(), extension_codec)?;

        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::Repartition(Box::new(
                protobuf::RepartitionExecNode {
                    input: Some(Box::new(input)),
                    partitioning: Some(pb_partitioning),
                },
            ))),
        })
    }

    fn try_from_sort_exec(
        exec: &SortExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let input = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.input().to_owned(),
            extension_codec,
        )?;
        let expr = exec
            .expr()
            .iter()
            .map(|expr| {
                let sort_expr = Box::new(protobuf::PhysicalSortExprNode {
                    expr: Some(Box::new(serialize_physical_expr(
                        &expr.expr,
                        extension_codec,
                    )?)),
                    asc: !expr.options.descending,
                    nulls_first: expr.options.nulls_first,
                });
                Ok(protobuf::PhysicalExprNode {
                    expr_type: Some(ExprType::Sort(sort_expr)),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::Sort(Box::new(
                protobuf::SortExecNode {
                    input: Some(Box::new(input)),
                    expr,
                    fetch: match exec.fetch() {
                        Some(n) => n as i64,
                        _ => -1,
                    },
                    preserve_partitioning: exec.preserve_partitioning(),
                },
            ))),
        })
    }

    fn try_from_union_exec(
        union: &UnionExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let mut inputs: Vec<protobuf::PhysicalPlanNode> = vec![];
        for input in union.inputs() {
            inputs.push(protobuf::PhysicalPlanNode::try_from_physical_plan(
                input.to_owned(),
                extension_codec,
            )?);
        }
        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::Union(protobuf::UnionExecNode {
                inputs,
            })),
        })
    }

    fn try_from_interleave_exec(
        interleave: &InterleaveExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let mut inputs: Vec<protobuf::PhysicalPlanNode> = vec![];
        for input in interleave.inputs() {
            inputs.push(protobuf::PhysicalPlanNode::try_from_physical_plan(
                input.to_owned(),
                extension_codec,
            )?);
        }
        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::Interleave(
                protobuf::InterleaveExecNode { inputs },
            )),
        })
    }

    fn try_from_sort_preserving_merge_exec(
        exec: &SortPreservingMergeExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let input = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.input().to_owned(),
            extension_codec,
        )?;
        let expr = exec
            .expr()
            .iter()
            .map(|expr| {
                let sort_expr = Box::new(protobuf::PhysicalSortExprNode {
                    expr: Some(Box::new(serialize_physical_expr(
                        &expr.expr,
                        extension_codec,
                    )?)),
                    asc: !expr.options.descending,
                    nulls_first: expr.options.nulls_first,
                });
                Ok(protobuf::PhysicalExprNode {
                    expr_type: Some(ExprType::Sort(sort_expr)),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::SortPreservingMerge(Box::new(
                protobuf::SortPreservingMergeExecNode {
                    input: Some(Box::new(input)),
                    expr,
                    fetch: exec.fetch().map(|f| f as i64).unwrap_or(-1),
                },
            ))),
        })
    }

    fn try_from_nested_loop_join_exec(
        exec: &NestedLoopJoinExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let left = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.left().to_owned(),
            extension_codec,
        )?;
        let right = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.right().to_owned(),
            extension_codec,
        )?;

        let join_type: protobuf::JoinType = exec.join_type().to_owned().into();
        let filter = exec
            .filter()
            .as_ref()
            .map(|f| {
                let expression =
                    serialize_physical_expr(f.expression(), extension_codec)?;
                let column_indices = f
                    .column_indices()
                    .iter()
                    .map(|i| {
                        let side: protobuf::JoinSide = i.side.to_owned().into();
                        protobuf::ColumnIndex {
                            index: i.index as u32,
                            side: side.into(),
                        }
                    })
                    .collect();
                let schema = f.schema().as_ref().try_into()?;
                Ok(protobuf::JoinFilter {
                    expression: Some(expression),
                    column_indices,
                    schema: Some(schema),
                })
            })
            .map_or(Ok(None), |v: Result<protobuf::JoinFilter>| v.map(Some))?;

        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::NestedLoopJoin(Box::new(
                protobuf::NestedLoopJoinExecNode {
                    left: Some(Box::new(left)),
                    right: Some(Box::new(right)),
                    join_type: join_type.into(),
                    filter,
                    projection: exec.projection().map_or_else(Vec::new, |v| {
                        v.iter().map(|x| *x as u32).collect::<Vec<u32>>()
                    }),
                },
            ))),
        })
    }

    fn try_from_window_agg_exec(
        exec: &WindowAggExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let input = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.input().to_owned(),
            extension_codec,
        )?;

        let window_expr = exec
            .window_expr()
            .iter()
            .map(|e| serialize_physical_window_expr(e, extension_codec))
            .collect::<Result<Vec<protobuf::PhysicalWindowExprNode>>>()?;

        let partition_keys = exec
            .partition_keys()
            .iter()
            .map(|e| serialize_physical_expr(e, extension_codec))
            .collect::<Result<Vec<protobuf::PhysicalExprNode>>>()?;

        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::Window(Box::new(
                protobuf::WindowAggExecNode {
                    input: Some(Box::new(input)),
                    window_expr,
                    partition_keys,
                    input_order_mode: None,
                },
            ))),
        })
    }

    fn try_from_bounded_window_agg_exec(
        exec: &BoundedWindowAggExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let input = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.input().to_owned(),
            extension_codec,
        )?;

        let window_expr = exec
            .window_expr()
            .iter()
            .map(|e| serialize_physical_window_expr(e, extension_codec))
            .collect::<Result<Vec<protobuf::PhysicalWindowExprNode>>>()?;

        let partition_keys = exec
            .partition_keys()
            .iter()
            .map(|e| serialize_physical_expr(e, extension_codec))
            .collect::<Result<Vec<protobuf::PhysicalExprNode>>>()?;

        let input_order_mode = match &exec.input_order_mode {
            InputOrderMode::Linear => {
                window_agg_exec_node::InputOrderMode::Linear(protobuf::EmptyMessage {})
            }
            InputOrderMode::PartiallySorted(columns) => {
                window_agg_exec_node::InputOrderMode::PartiallySorted(
                    protobuf::PartiallySortedInputOrderMode {
                        columns: columns.iter().map(|c| *c as u64).collect(),
                    },
                )
            }
            InputOrderMode::Sorted => {
                window_agg_exec_node::InputOrderMode::Sorted(protobuf::EmptyMessage {})
            }
        };

        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::Window(Box::new(
                protobuf::WindowAggExecNode {
                    input: Some(Box::new(input)),
                    window_expr,
                    partition_keys,
                    input_order_mode: Some(input_order_mode),
                },
            ))),
        })
    }

    fn try_from_data_sink_exec(
        exec: &DataSinkExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Option<Self>> {
        let input: protobuf::PhysicalPlanNode =
            protobuf::PhysicalPlanNode::try_from_physical_plan(
                exec.input().to_owned(),
                extension_codec,
            )?;
        let sort_order = match exec.sort_order() {
            Some(requirements) => {
                let expr = requirements
                    .iter()
                    .map(|requirement| {
                        let expr: PhysicalSortExpr = requirement.to_owned().into();
                        let sort_expr = protobuf::PhysicalSortExprNode {
                            expr: Some(Box::new(serialize_physical_expr(
                                &expr.expr,
                                extension_codec,
                            )?)),
                            asc: !expr.options.descending,
                            nulls_first: expr.options.nulls_first,
                        };
                        Ok(sort_expr)
                    })
                    .collect::<Result<Vec<_>>>()?;
                Some(protobuf::PhysicalSortExprNodeCollection {
                    physical_sort_expr_nodes: expr,
                })
            }
            None => None,
        };

        if let Some(sink) = exec.sink().as_any().downcast_ref::<JsonSink>() {
            return Ok(Some(protobuf::PhysicalPlanNode {
                physical_plan_type: Some(PhysicalPlanType::JsonSink(Box::new(
                    protobuf::JsonSinkExecNode {
                        input: Some(Box::new(input)),
                        sink: Some(sink.try_into()?),
                        sink_schema: Some(exec.schema().as_ref().try_into()?),
                        sort_order,
                    },
                ))),
            }));
        }

        if let Some(sink) = exec.sink().as_any().downcast_ref::<CsvSink>() {
            return Ok(Some(protobuf::PhysicalPlanNode {
                physical_plan_type: Some(PhysicalPlanType::CsvSink(Box::new(
                    protobuf::CsvSinkExecNode {
                        input: Some(Box::new(input)),
                        sink: Some(sink.try_into()?),
                        sink_schema: Some(exec.schema().as_ref().try_into()?),
                        sort_order,
                    },
                ))),
            }));
        }

        #[cfg(feature = "parquet")]
        if let Some(sink) = exec.sink().as_any().downcast_ref::<ParquetSink>() {
            return Ok(Some(protobuf::PhysicalPlanNode {
                physical_plan_type: Some(PhysicalPlanType::ParquetSink(Box::new(
                    protobuf::ParquetSinkExecNode {
                        input: Some(Box::new(input)),
                        sink: Some(sink.try_into()?),
                        sink_schema: Some(exec.schema().as_ref().try_into()?),
                        sort_order,
                    },
                ))),
            }));
        }

        // If unknown DataSink then let extension handle it
        Ok(None)
    }

    fn try_from_unnest_exec(
        exec: &UnnestExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let input = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.input().to_owned(),
            extension_codec,
        )?;

        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::Unnest(Box::new(
                protobuf::UnnestExecNode {
                    input: Some(Box::new(input)),
                    schema: Some(exec.schema().try_into()?),
                    list_type_columns: exec
                        .list_column_indices()
                        .iter()
                        .map(|c| ProtoListUnnest {
                            index_in_input_schema: c.index_in_input_schema as _,
                            depth: c.depth as _,
                        })
                        .collect(),
                    struct_type_columns: exec
                        .struct_column_indices()
                        .iter()
                        .map(|c| *c as _)
                        .collect(),
                    options: Some(exec.options().into()),
                },
            ))),
        })
    }

    fn try_from_cooperative_exec(
        exec: &CooperativeExec,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self> {
        let input = protobuf::PhysicalPlanNode::try_from_physical_plan(
            exec.input().to_owned(),
            extension_codec,
        )?;

        Ok(protobuf::PhysicalPlanNode {
            physical_plan_type: Some(PhysicalPlanType::Cooperative(Box::new(
                protobuf::CooperativeExecNode {
                    input: Some(Box::new(input)),
                },
            ))),
        })
    }
}

pub trait AsExecutionPlan: Debug + Send + Sync + Clone {
    fn try_decode(buf: &[u8]) -> Result<Self>
    where
        Self: Sized;

    fn try_encode<B>(&self, buf: &mut B) -> Result<()>
    where
        B: BufMut,
        Self: Sized;

    fn try_into_physical_plan(
        &self,
        registry: &dyn FunctionRegistry,
        runtime: &RuntimeEnv,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Arc<dyn ExecutionPlan>>;

    fn try_from_physical_plan(
        plan: Arc<dyn ExecutionPlan>,
        extension_codec: &dyn PhysicalExtensionCodec,
    ) -> Result<Self>
    where
        Self: Sized;
}

pub trait PhysicalExtensionCodec: Debug + Send + Sync {
    fn try_decode(
        &self,
        buf: &[u8],
        inputs: &[Arc<dyn ExecutionPlan>],
        registry: &dyn FunctionRegistry,
    ) -> Result<Arc<dyn ExecutionPlan>>;

    fn try_encode(&self, node: Arc<dyn ExecutionPlan>, buf: &mut Vec<u8>) -> Result<()>;

    fn try_decode_udf(&self, name: &str, _buf: &[u8]) -> Result<Arc<ScalarUDF>> {
        not_impl_err!("PhysicalExtensionCodec is not provided for scalar function {name}")
    }

    fn try_encode_udf(&self, _node: &ScalarUDF, _buf: &mut Vec<u8>) -> Result<()> {
        Ok(())
    }

    fn try_decode_expr(
        &self,
        _buf: &[u8],
        _inputs: &[Arc<dyn PhysicalExpr>],
    ) -> Result<Arc<dyn PhysicalExpr>> {
        not_impl_err!("PhysicalExtensionCodec is not provided")
    }

    fn try_encode_expr(
        &self,
        _node: &Arc<dyn PhysicalExpr>,
        _buf: &mut Vec<u8>,
    ) -> Result<()> {
        not_impl_err!("PhysicalExtensionCodec is not provided")
    }

    fn try_decode_udaf(&self, name: &str, _buf: &[u8]) -> Result<Arc<AggregateUDF>> {
        not_impl_err!(
            "PhysicalExtensionCodec is not provided for aggregate function {name}"
        )
    }

    fn try_encode_udaf(&self, _node: &AggregateUDF, _buf: &mut Vec<u8>) -> Result<()> {
        Ok(())
    }

    fn try_decode_udwf(&self, name: &str, _buf: &[u8]) -> Result<Arc<WindowUDF>> {
        not_impl_err!("PhysicalExtensionCodec is not provided for window function {name}")
    }

    fn try_encode_udwf(&self, _node: &WindowUDF, _buf: &mut Vec<u8>) -> Result<()> {
        Ok(())
    }
}

#[derive(Debug)]
pub struct DefaultPhysicalExtensionCodec {}

impl PhysicalExtensionCodec for DefaultPhysicalExtensionCodec {
    fn try_decode(
        &self,
        _buf: &[u8],
        _inputs: &[Arc<dyn ExecutionPlan>],
        _registry: &dyn FunctionRegistry,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        not_impl_err!("PhysicalExtensionCodec is not provided")
    }

    fn try_encode(
        &self,
        _node: Arc<dyn ExecutionPlan>,
        _buf: &mut Vec<u8>,
    ) -> Result<()> {
        not_impl_err!("PhysicalExtensionCodec is not provided")
    }
}

/// DataEncoderTuple captures the position of the encoder
/// in the codec list that was used to encode the data and actual encoded data
#[derive(Clone, PartialEq, prost::Message)]
struct DataEncoderTuple {
    /// The position of encoder used to encode data
    /// (to be used for decoding)
    #[prost(uint32, tag = 1)]
    pub encoder_position: u32,

    #[prost(bytes, tag = 2)]
    pub blob: Vec<u8>,
}

/// A PhysicalExtensionCodec that tries one of multiple inner codecs
/// until one works
#[derive(Debug)]
pub struct ComposedPhysicalExtensionCodec {
    codecs: Vec<Arc<dyn PhysicalExtensionCodec>>,
}

impl ComposedPhysicalExtensionCodec {
    // Position in this codesc list is important as it will be used for decoding.
    // If new codec is added it should go to last position.
    pub fn new(codecs: Vec<Arc<dyn PhysicalExtensionCodec>>) -> Self {
        Self { codecs }
    }

    fn decode_protobuf<R>(
        &self,
        buf: &[u8],
        decode: impl FnOnce(&dyn PhysicalExtensionCodec, &[u8]) -> Result<R>,
    ) -> Result<R> {
        let proto = DataEncoderTuple::decode(buf)
            .map_err(|e| DataFusionError::Internal(e.to_string()))?;

        let codec = self.codecs.get(proto.encoder_position as usize).ok_or(
            DataFusionError::Internal(
                "Can't find required codec in codec list".to_owned(),
            ),
        )?;

        decode(codec.as_ref(), &proto.blob)
    }

    fn encode_protobuf(
        &self,
        buf: &mut Vec<u8>,
        mut encode: impl FnMut(&dyn PhysicalExtensionCodec, &mut Vec<u8>) -> Result<()>,
    ) -> Result<()> {
        let mut data = vec![];
        let mut last_err = None;
        let mut encoder_position = None;

        // find the encoder
        for (position, codec) in self.codecs.iter().enumerate() {
            match encode(codec.as_ref(), &mut data) {
                Ok(_) => {
                    encoder_position = Some(position as u32);
                    break;
                }
                Err(err) => last_err = Some(err),
            }
        }

        let encoder_position = encoder_position.ok_or_else(|| {
            last_err.unwrap_or_else(|| {
                DataFusionError::NotImplemented(
                    "Empty list of composed codecs".to_owned(),
                )
            })
        })?;

        // encode with encoder position
        let proto = DataEncoderTuple {
            encoder_position,
            blob: data,
        };
        proto
            .encode(buf)
            .map_err(|e| DataFusionError::Internal(e.to_string()))
    }
}

impl PhysicalExtensionCodec for ComposedPhysicalExtensionCodec {
    fn try_decode(
        &self,
        buf: &[u8],
        inputs: &[Arc<dyn ExecutionPlan>],
        registry: &dyn FunctionRegistry,
    ) -> Result<Arc<dyn ExecutionPlan>> {
        self.decode_protobuf(buf, |codec, data| codec.try_decode(data, inputs, registry))
    }

    fn try_encode(&self, node: Arc<dyn ExecutionPlan>, buf: &mut Vec<u8>) -> Result<()> {
        self.encode_protobuf(buf, |codec, data| codec.try_encode(Arc::clone(&node), data))
    }

    fn try_decode_udf(&self, name: &str, buf: &[u8]) -> Result<Arc<ScalarUDF>> {
        self.decode_protobuf(buf, |codec, data| codec.try_decode_udf(name, data))
    }

    fn try_encode_udf(&self, node: &ScalarUDF, buf: &mut Vec<u8>) -> Result<()> {
        self.encode_protobuf(buf, |codec, data| codec.try_encode_udf(node, data))
    }

    fn try_decode_udaf(&self, name: &str, buf: &[u8]) -> Result<Arc<AggregateUDF>> {
        self.decode_protobuf(buf, |codec, data| codec.try_decode_udaf(name, data))
    }

    fn try_encode_udaf(&self, node: &AggregateUDF, buf: &mut Vec<u8>) -> Result<()> {
        self.encode_protobuf(buf, |codec, data| codec.try_encode_udaf(node, data))
    }
}

fn into_physical_plan(
    node: &Option<Box<protobuf::PhysicalPlanNode>>,
    registry: &dyn FunctionRegistry,
    runtime: &RuntimeEnv,
    extension_codec: &dyn PhysicalExtensionCodec,
) -> Result<Arc<dyn ExecutionPlan>> {
    if let Some(field) = node {
        field.try_into_physical_plan(registry, runtime, extension_codec)
    } else {
        Err(proto_error("Missing required field in protobuf"))
    }
}
